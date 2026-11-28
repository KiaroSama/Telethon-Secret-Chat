use std::collections::BTreeSet;

use serde_json::json;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{ChatCheckpoint, CoreSecretChatManager, OutboundMessage, SecretTransport};
use crate::errors::SecretChatError;
use crate::protocol::{MAX_ENCRYPTED_FRAME_LEN, encrypt_frame, outgoing_layer, wrap_frame};

#[derive(Clone, Copy)]
pub(crate) enum TextTtl {
    Explicit(i64),
    Conversation,
}

impl<T: SecretTransport> CoreSecretChatManager<T> {
    pub async fn send_text(
        &self,
        chat_id: i64,
        random_id: i64,
        text: &str,
        ttl: i64,
    ) -> Result<Vec<u8>, SecretChatError> {
        self.send_text_checkpointed(chat_id, random_id, text, TextTtl::Explicit(ttl), None)
            .await
    }

    pub(crate) async fn send_text_checkpointed(
        &self,
        chat_id: i64,
        random_id: i64,
        text: &str,
        ttl_policy: TextTtl,
        checkpoint: Option<&dyn ChatCheckpoint>,
    ) -> Result<Vec<u8>, SecretChatError> {
        self.ensure_running()?;
        if matches!(ttl_policy, TextTtl::Explicit(ttl) if !(0..=i64::from(i32::MAX)).contains(&ttl))
            || text.len() > self.limits.max_retained_bytes
            || text.len() > MAX_ENCRYPTED_FRAME_LEN
        {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "message TTL or size exceeds its supported bound".into(),
            });
        }
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.ensure_running()?;
        let identity = match ttl_policy {
            TextTtl::Explicit(ttl) => text_identity(text, ttl),
            TextTtl::Conversation => conversation_identity(text),
        };

        let existing = {
            let state = entry.state.lock().await;
            self.require_ready(&state.chat)?;
            if let Some((sequence, record)) = state
                .outbox
                .iter()
                .find(|(_, record)| record.random_id == random_id)
            {
                let matches = if record.service {
                    false
                } else if record.identity == identity {
                    true
                } else if matches!(ttl_policy, TextTtl::Conversation) {
                    // Older implicit sends used the explicit TTL/text digest. Preserve it.
                    record.identity == text_identity(text, i64::from(state.chat.ttl()))
                        || state
                            .chat
                            .retained_outgoing_text_ttl(&record.frame, *sequence, random_id, text)?
                            .is_some_and(|ttl| record.identity == text_identity(text, ttl))
                } else {
                    false
                };
                if !matches {
                    return Err(SecretChatError::ParameterRejected {
                        chat_id: Some(chat_id),
                        reason: "random_id is already bound to different message content".into(),
                    });
                }
                Some(record.frame.to_vec())
            } else {
                None
            }
        };

        // Retried requests retain the same ciphertext, padding, random_id, and sequence.
        self.transmit_pending(chat_id, &entry, None, checkpoint)
            .await?;
        if let Some(frame) = existing {
            self.transmit_pending(chat_id, &entry, Some(order), checkpoint)
                .await?;
            return Ok(frame);
        }

        let frame = {
            let mut state = entry.state.lock().await;
            self.ensure_running()?;
            self.require_ready(&state.chat)?;
            let mut candidate = state.chat.clone();
            let mut services = Vec::new();
            if candidate.rekey_due() {
                match candidate.start_rekey() {
                    Ok(Some(service)) => services.push(service),
                    Ok(None) => (),
                    Err(error) => {
                        self.commit_fatal_state(chat_id, &mut state, &candidate, checkpoint)?;
                        return Err(error);
                    }
                }
            }
            let chat = &candidate;
            // Implicit policy is resolved only when this new frame reserves its sequence.
            // Draining an older outbox above may have awaited authenticated TTL updates.
            let ttl = match ttl_policy {
                TextTtl::Explicit(ttl) => ttl,
                TextTtl::Conversation => i64::from(chat.ttl()),
            };
            let sequence = chat.out_seq_no();
            if sequence < 0 || sequence > i64::from(i32::MAX) / 2 {
                return Err(SecretChatError::ParameterRejected {
                    chat_id: Some(chat_id),
                    reason: "outgoing sequence space exhausted".into(),
                });
            }
            let layer = outgoing_layer(chat.negotiated_layer())?;
            let in_wire = chat.in_seq_no() * 2 + i64::from(!chat.is_outbound());
            let out_wire = sequence * 2 + i64::from(chat.is_outbound());
            let body = Zeroizing::new(wrap_frame(
                &json!({
                    "random_id": random_id, "ttl": ttl, "message": text,
                }),
                layer,
                in_wire,
                out_wire,
            )?);
            let key = Zeroizing::new(chat.export_key_for_internal_frame().ok_or(
                SecretChatError::ChatNotReady {
                    chat_id,
                    state: "key unavailable".into(),
                },
            )?);
            let frame = encrypt_frame(&*key, &body, chat.out_x())?;
            if frame.len() > MAX_ENCRYPTED_FRAME_LEN {
                return Err(SecretChatError::ParameterRejected {
                    chat_id: Some(chat_id),
                    reason: "encrypted message exceeds the frame resource limit".into(),
                });
            }
            candidate.set_out_seq_no(sequence + 1);
            self.stage_due_rekey(
                chat_id,
                &mut state,
                &mut candidate,
                &mut services,
                checkpoint,
            )?;
            self.check_capacity(
                chat_id,
                &state,
                &BTreeSet::new(),
                services.len() + 1,
                services
                    .iter()
                    .map(|service| service.frame.len())
                    .sum::<usize>()
                    + frame.len(),
            )?;
            let mut next = state.clone();
            self.retain_services(&mut next, services);
            next.outbox.insert(
                sequence,
                OutboundMessage {
                    random_id,
                    identity,
                    frame: Zeroizing::new(frame.clone()),
                    pending: true,
                    service: false,
                    replay_requested: false,
                    acknowledged: false,
                    revision: 0,
                },
            );
            // Reserve before awaiting the transport: cancellation cannot roll back possible delivery.
            next.chat = candidate;
            self.commit_candidate(&mut state, next, checkpoint, None)?;
            frame
        };
        self.transmit_pending(chat_id, &entry, Some(order), checkpoint)
            .await?;
        Ok(frame)
    }
}

fn text_identity(text: &str, ttl: i64) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"secret-chat-text\0");
    hash.update(ttl.to_le_bytes());
    hash.update(text.as_bytes());
    hash.finalize().into()
}

fn conversation_identity(text: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"secret-chat-conversation-text\0");
    hash.update(text.as_bytes());
    hash.finalize().into()
}
