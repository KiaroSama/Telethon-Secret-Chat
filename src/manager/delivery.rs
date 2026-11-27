use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::MutexGuard;
use zeroize::Zeroizing;

use super::{
    ChatEntry, ManagedChat, OutboundMessage, SecretChatManager, SecretEvent, SecretTransport,
};
use crate::errors::SecretChatError;
use crate::protocol::{AcceptedMessage, ChatState, PreparedServiceMessage, SecretChat};

impl<T: SecretTransport> SecretChatManager<T> {
    pub async fn receive_frame(
        &self,
        chat_id: i64,
        frame: &[u8],
    ) -> Result<AcceptedMessage, SecretChatError> {
        self.receive_frames(chat_id, frame)
            .await?
            .into_iter()
            .next()
            .ok_or(SecretChatError::MessageRejected {
                chat_id: Some(chat_id),
                reason:
                    "frame was buffered or already processed; use receive_frames for batch outcomes"
                        .into(),
            })
    }

    /// Accept a frame and publish every recovered message in conversation order.
    /// An empty batch means a safely buffered gap or an already processed message.
    pub async fn receive_frames(
        &self,
        chat_id: i64,
        frame: &[u8],
    ) -> Result<Vec<AcceptedMessage>, SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let mut state = entry.state.lock().await;
        self.ensure_running()?;
        let mut candidate = state.chat.clone();
        let accepted = match candidate.receive_encrypted_frames(frame) {
            Ok(accepted) => accepted,
            Err(error) => {
                self.commit_fatal_state(chat_id, &mut state, &candidate);
                return Err(error);
            }
        };
        let mut replay = BTreeSet::new();
        for action in candidate.take_immediate_actions() {
            let start = action["fields"]["start_seq_no"].as_i64().unwrap_or(-1);
            let end = action["fields"]["end_seq_no"].as_i64().unwrap_or(-1);
            let parity = i64::from(candidate.is_outbound());
            let valid = start >= 0
                && end >= start
                && start % 2 == parity
                && end % 2 == parity
                && (end - start) / 2 < 1000;
            let range = if valid {
                (start / 2..=end / 2).collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if !valid || range.iter().any(|seq| !state.outbox.contains_key(seq)) {
                let retained_from = state
                    .outbox
                    .first_key_value()
                    .map_or(-1, |(seq, _)| seq * 2 + parity);
                candidate.close();
                self.commit_fatal_state(chat_id, &mut state, &candidate);
                return Err(SecretChatError::ResendUnsatisfiable {
                    chat_id,
                    requested: (start, end),
                    retained_from,
                });
            }
            replay.extend(range);
        }
        let mut services = Vec::new();
        for message in &accepted {
            if let Some(action) = &message.action {
                match candidate.handle_rekey_action(action) {
                    Ok(replies) => services.extend(replies),
                    Err(error) => {
                        self.commit_fatal_state(chat_id, &mut state, &candidate);
                        return Err(error);
                    }
                }
            }
        }
        if candidate.take_pending_noop() {
            services
                .push(candidate.prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))?);
        }
        if let Some((start, end)) = candidate.take_resend_request() {
            services.push(candidate.prepare_service_action(
                &json!({"id":0x511110b0_u32,"fields":{"start_seq_no":start,"end_seq_no":end}}),
            )?);
        }
        self.stage_due_rekey(chat_id, &mut state, &mut candidate, &mut services)?;
        let acknowledged = candidate.peer_in_seq_no();
        let remove: BTreeSet<_> = state
            .outbox
            .iter()
            .filter_map(|(seq, record)| {
                (*seq < acknowledged && !record.replay_requested && !replay.contains(seq))
                    .then_some(*seq)
            })
            .collect();
        self.check_capacity(
            chat_id,
            &state,
            &remove,
            services.len(),
            services.iter().map(|s| s.frame.len()).sum(),
        )?;
        for (seq, record) in &mut state.outbox {
            if *seq < acknowledged && !record.acknowledged {
                self.emit(SecretEvent::MessageAcknowledged {
                    chat_id,
                    random_id: record.random_id,
                });
                record.acknowledged = true;
            }
            if replay.contains(seq) {
                record.pending = true;
                record.replay_requested = true;
                record.revision = record.revision.wrapping_add(1);
            }
        }
        state.outbox.retain(|seq, _| !remove.contains(seq));
        self.retain_services(&mut state, services);
        state.chat = candidate;
        // No cancellation point is permitted between accepting a message and queuing its event.
        for message in &accepted {
            self.emit(if message.action.is_some() {
                SecretEvent::ServiceActionReceived {
                    chat_id,
                    message: message.clone(),
                }
            } else {
                SecretEvent::MessageReceived {
                    chat_id,
                    message: message.clone(),
                }
            });
        }
        drop(state);
        // A transport may deliver an update inline during send. Never wait on its own send lock.
        // The lock holder will see newly queued responses on its next transmit iteration.
        if let Ok(order) = entry.outbound.try_lock() {
            self.transmit_pending(chat_id, &entry, Some(order)).await?;
        }
        Ok(accepted)
    }

    pub async fn close_chat(
        &self,
        chat_id: i64,
        delete_history: bool,
        reason: &str,
    ) -> Result<(), SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let _order = entry.outbound.lock().await;
        self.ensure_running()?;
        {
            let mut state = entry.state.lock().await;
            self.ensure_running()?;
            if state.chat.state() != ChatState::Closed {
                state.chat.close();
                state.outbox.clear();
                self.emit(SecretEvent::ChatClosed {
                    chat_id,
                    reason: reason.to_owned(),
                });
            }
        }
        self.transport
            .discard_encryption(chat_id, delete_history)
            .await
            .map_err(|_| SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "transport discard failed after local closure".into(),
            })
    }

    /// Stop new work and erase local state. Caller-owned in-flight RPCs may settle afterwards.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let entries: Vec<_> = self.chats.lock().await.values().cloned().collect();
        for entry in entries {
            let mut state = entry.state.lock().await;
            state.chat.close();
            state.outbox.clear();
        }
    }

    pub async fn drain_events(&self) -> Vec<SecretEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub async fn chat_state(&self, chat_id: i64) -> Result<ChatState, SecretChatError> {
        Ok(self.entry(chat_id).await?.state.lock().await.chat.state())
    }

    pub async fn out_seq_no(&self, chat_id: i64) -> Result<i64, SecretChatError> {
        Ok(self
            .entry(chat_id)
            .await?
            .state
            .lock()
            .await
            .chat
            .out_seq_no())
    }

    pub(super) async fn entry(&self, chat_id: i64) -> Result<Arc<ChatEntry>, SecretChatError> {
        self.chats
            .lock()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or(SecretChatError::UnknownChat { chat_id })
    }

    pub(super) fn emit(&self, event: SecretEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    pub(super) fn ensure_running(&self) -> Result<(), SecretChatError> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(SecretChatError::ManagerStopping);
        }
        Ok(())
    }

    pub(super) fn require_ready(&self, chat: &SecretChat) -> Result<(), SecretChatError> {
        match chat.state() {
            ChatState::Ready => Ok(()),
            ChatState::Closed => Err(SecretChatError::ChatClosed {
                chat_id: chat.chat_id(),
                reason: "chat is closed".into(),
            }),
            state => Err(SecretChatError::ChatNotReady {
                chat_id: chat.chat_id(),
                state: format!("{state:?}").to_lowercase(),
            }),
        }
    }

    pub(super) fn check_capacity(
        &self,
        chat_id: i64,
        state: &ManagedChat,
        remove: &BTreeSet<i64>,
        additional_count: usize,
        additional_bytes: usize,
    ) -> Result<(), SecretChatError> {
        let retained: usize = state
            .outbox
            .iter()
            .filter(|(seq, _)| !remove.contains(seq))
            .map(|(_, record)| record.frame.len())
            .sum();
        if additional_count
            > self
                .limits
                .max_retained_messages
                .saturating_sub(state.outbox.len() - remove.len())
            || additional_bytes > self.limits.max_retained_bytes.saturating_sub(retained)
        {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "retained outbox budget exhausted; await peer acknowledgement".into(),
            });
        }
        Ok(())
    }

    pub(super) fn retain_services(
        &self,
        state: &mut ManagedChat,
        services: Vec<PreparedServiceMessage>,
    ) {
        for service in services {
            state.outbox.insert(
                service.out_seq_no,
                OutboundMessage {
                    random_id: service.random_id,
                    identity: [0; 32],
                    frame: Zeroizing::new(service.frame),
                    pending: true,
                    service: true,
                    replay_requested: false,
                    acknowledged: false,
                    revision: 0,
                },
            );
        }
    }

    pub(super) fn stage_due_rekey(
        &self,
        chat_id: i64,
        state: &mut ManagedChat,
        candidate: &mut SecretChat,
        services: &mut Vec<PreparedServiceMessage>,
    ) -> Result<(), SecretChatError> {
        if candidate.rekey_due() {
            match candidate.start_rekey() {
                Ok(Some(service)) => services.push(service),
                Ok(None) => (),
                Err(error) => {
                    self.commit_fatal_state(chat_id, state, candidate);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) fn commit_fatal_state(
        &self,
        chat_id: i64,
        state: &mut ManagedChat,
        candidate: &SecretChat,
    ) {
        if candidate.state() == ChatState::Closed && state.chat.state() != ChatState::Closed {
            state.chat.close();
            state.outbox.clear();
            self.emit(SecretEvent::ChatClosed {
                chat_id,
                reason: "authenticated protocol or key lifecycle violation".into(),
            });
        }
    }

    pub(super) async fn transmit_pending(
        &self,
        chat_id: i64,
        entry: &ChatEntry,
        mut final_order: Option<MutexGuard<'_, ()>>,
    ) -> Result<usize, SecretChatError> {
        let mut count = 0;
        loop {
            self.ensure_running()?;
            let pending = {
                let state = entry.state.lock().await;
                self.require_ready(&state.chat)?;
                let pending = state.outbox.iter().find(|(_, record)| record.pending).map(
                    |(sequence, record)| {
                        (
                            *sequence,
                            record.random_id,
                            record.frame.to_vec(),
                            record.service,
                            record.revision,
                        )
                    },
                );
                if pending.is_none() {
                    // Release under the state lock: a receiver can either enqueue before
                    // this check or acquire outbound after it, never miss both drivers.
                    drop(final_order.take());
                }
                pending
            };
            let Some((sequence, random_id, frame, service, revision)) = pending else {
                return Ok(count);
            };
            let sending = if service {
                self.transport
                    .send_encrypted_service(chat_id, random_id, frame)
            } else {
                self.transport.send_encrypted(chat_id, random_id, frame)
            };
            let result = sending.await;
            let mut state = entry.state.lock().await;
            if result.is_err() {
                // Authenticated peer settlement is stronger evidence than a lost RPC reply.
                // A replay obligation still needs its own successful transmission.
                if state.chat.peer_in_seq_no() > sequence && !state.outbox.contains_key(&sequence) {
                    count += 1;
                    continue;
                }
                return Err(SecretChatError::SendPending {
                    chat_id,
                    random_id,
                    cause: "TransportError".into(),
                });
            }
            if let Some(record) = state.outbox.get_mut(&sequence)
                && record.revision == revision
            {
                record.pending = false;
                record.replay_requested = false;
                if record.acknowledged {
                    state.outbox.remove(&sequence);
                }
            }
            count += 1;
        }
    }
}

pub(super) fn text_identity(text: &str, ttl: i64) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"secret-chat-text\0");
    hash.update(ttl.to_le_bytes());
    hash.update(text.as_bytes());
    hash.finalize().into()
}
