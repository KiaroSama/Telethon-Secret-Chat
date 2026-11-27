use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as EventMutex};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, MutexGuard};
use zeroize::Zeroizing;

use crate::errors::SecretChatError;
use crate::protocol::{
    AcceptedMessage, ChatRole, ChatState, MAX_ENCRYPTED_FRAME_LEN, PreparedServiceMessage,
    SecretChat, encrypt_frame, outgoing_layer, wrap_frame,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The caller supplies an authorized transport; the manager never logs transport errors.
pub trait SecretTransport: Send + Sync {
    fn auth_identity(&self) -> &str;
    fn send_encrypted<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>>;
    /// Implement using messages.sendEncryptedService, preserving the supplied ciphertext.
    /// The default refuses explicitly instead of sending a service with the wrong RPC.
    fn send_encrypted_service<'a>(
        &'a self,
        chat_id: i64,
        _random_id: i64,
        _frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "the transport must implement encrypted service delivery".into(),
            })
        })
    }
    fn discard_encryption<'a>(
        &'a self,
        chat_id: i64,
        delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretEvent {
    ChatReady {
        chat_id: i64,
    },
    MessageReceived {
        chat_id: i64,
        message: AcceptedMessage,
    },
    ServiceActionReceived {
        chat_id: i64,
        message: AcceptedMessage,
    },
    MessageAcknowledged {
        chat_id: i64,
        random_id: i64,
    },
    ChatClosed {
        chat_id: i64,
        reason: String,
    },
}

/// Finite retained-outbox budgets. Exhaustion refuses new sends without evicting data.
#[derive(Debug, Clone, Copy)]
pub struct ManagerLimits {
    pub max_retained_messages: usize,
    pub max_retained_bytes: usize,
}

impl Default for ManagerLimits {
    fn default() -> Self {
        Self {
            max_retained_messages: 1024,
            max_retained_bytes: 16 * 1024 * 1024,
        }
    }
}

struct OutboundMessage {
    random_id: i64,
    identity: [u8; 32],
    frame: Zeroizing<Vec<u8>>,
    pending: bool,
    service: bool,
    replay_requested: bool,
    acknowledged: bool,
    revision: u64,
}

struct ManagedChat {
    chat: SecretChat,
    outbox: BTreeMap<i64, OutboundMessage>,
}

struct ChatEntry {
    state: Mutex<ManagedChat>,
    outbound: Mutex<()>,
}

/// An in-memory manager. Durable stores are separate APIs, not an automatic restart guarantee.
/// Per-chat outbound locks serialize RPCs while allowing receive processing during an await.
pub struct SecretChatManager<T: SecretTransport> {
    transport: Arc<T>,
    chats: Mutex<BTreeMap<i64, Arc<ChatEntry>>>,
    events: EventMutex<Vec<SecretEvent>>,
    stopping: AtomicBool,
    limits: ManagerLimits,
}

impl<T: SecretTransport> SecretChatManager<T> {
    pub fn new(transport: Arc<T>) -> Self {
        Self {
            transport,
            chats: Mutex::new(BTreeMap::new()),
            events: EventMutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
            limits: ManagerLimits::default(),
        }
    }

    pub fn with_limits(transport: Arc<T>, limits: ManagerLimits) -> Result<Self, SecretChatError> {
        if limits.max_retained_messages == 0 || limits.max_retained_bytes == 0 {
            return Err(SecretChatError::ParameterRejected {
                chat_id: None,
                reason: "manager retention limits must be positive".into(),
            });
        }
        Ok(Self {
            limits,
            ..Self::new(transport)
        })
    }

    pub fn caller_auth_identity(&self) -> &str {
        self.transport.auth_identity()
    }

    pub async fn register_established_chat(
        &self,
        chat_id: i64,
        access_hash: i64,
        peer_user_id: i64,
        role: ChatRole,
        shared_key: &[u8],
    ) -> Result<i64, SecretChatError> {
        self.ensure_running()?;
        if self.transport.auth_identity().trim().is_empty() {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "caller authorization identity must not be empty".into(),
            });
        }
        let mut chats = self.chats.lock().await;
        self.ensure_running()?;
        if chats.contains_key(&chat_id) {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "chat identity is already registered or tombstoned".into(),
            });
        }
        let mut chat = SecretChat::new(
            chat_id,
            access_hash,
            peer_user_id,
            self.transport.auth_identity(),
            role,
        );
        let fp = chat.adopt_key(shared_key)?;
        chats.insert(
            chat_id,
            Arc::new(ChatEntry {
                state: Mutex::new(ManagedChat {
                    chat,
                    outbox: BTreeMap::new(),
                }),
                outbound: Mutex::new(()),
            }),
        );
        self.emit(SecretEvent::ChatReady { chat_id });
        Ok(fp)
    }

    pub async fn send_text(
        &self,
        chat_id: i64,
        random_id: i64,
        text: &str,
        ttl: i64,
    ) -> Result<Vec<u8>, SecretChatError> {
        self.ensure_running()?;
        if !(0..=i64::from(i32::MAX)).contains(&ttl)
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
        let identity = text_identity(text, ttl);

        let existing = {
            let state = entry.state.lock().await;
            self.require_ready(&state.chat)?;
            if let Some(record) = state
                .outbox
                .values()
                .find(|record| record.random_id == random_id)
            {
                if record.service || record.identity != identity {
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
        self.transmit_pending(chat_id, &entry, None).await?;
        if let Some(frame) = existing {
            self.transmit_pending(chat_id, &entry, Some(order)).await?;
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
                        self.commit_fatal_state(chat_id, &mut state, &candidate);
                        return Err(error);
                    }
                }
            }
            let chat = &candidate;
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
            self.stage_due_rekey(chat_id, &mut state, &mut candidate, &mut services)?;
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
            self.retain_services(&mut state, services);
            state.outbox.insert(
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
            state.chat = candidate;
            frame
        };
        self.transmit_pending(chat_id, &entry, Some(order)).await?;
        Ok(frame)
    }

    /// Retry unsettled transmissions; successful RPCs remain retained until peer acknowledgement.
    pub async fn retry_pending(&self, chat_id: i64) -> Result<usize, SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.transmit_pending(chat_id, &entry, Some(order)).await
    }

    /// Supply the validated original DH parameters before the automatic rekey threshold.
    pub async fn configure_chat_dh(
        &self,
        chat_id: i64,
        g: i64,
        p: &[u8],
    ) -> Result<(), SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let mut state = entry.state.lock().await;
        self.ensure_running()?;
        state.chat.configure_dh(g, p)
    }

    /// Begin a key exchange, retaining its exact service frame before any network await.
    pub async fn start_rekey(&self, chat_id: i64) -> Result<bool, SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.transmit_pending(chat_id, &entry, None).await?;
        let started = {
            let mut state = entry.state.lock().await;
            self.ensure_running()?;
            let mut candidate = state.chat.clone();
            let prepared = match candidate.start_rekey() {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.commit_fatal_state(chat_id, &mut state, &candidate);
                    return Err(error);
                }
            };
            if let Some(prepared) = prepared {
                self.check_capacity(chat_id, &state, &BTreeSet::new(), 1, prepared.frame.len())?;
                self.retain_services(&mut state, vec![prepared]);
                state.chat = candidate;
                true
            } else {
                false
            }
        };
        self.transmit_pending(chat_id, &entry, Some(order)).await?;
        Ok(started)
    }

    /// Send a conversation action. Rekey and recovery actions are owned by the state machine.
    /// TTL, deletion, reading and archive effects remain the application's responsibility.
    pub async fn send_service_action(
        &self,
        chat_id: i64,
        action: &Value,
    ) -> Result<Vec<u8>, SecretChatError> {
        let id = action["id"].as_u64().and_then(|id| u32::try_from(id).ok());
        if !matches!(
            id,
            Some(
                0xa1733aec
                    | 0x0c4f40be
                    | 0x65614304
                    | 0x8ac1f475
                    | 0x6719e45c
                    | 0xf3048883
                    | 0xccb27641
                    | 0xa82fdd63
            )
        ) || (id == Some(0xa1733aec)
            && action["fields"]["ttl_seconds"]
                .as_i64()
                .is_none_or(|ttl| ttl < 0))
        {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: "unsupported or manager-owned service action".into(),
            });
        }
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.transmit_pending(chat_id, &entry, None).await?;
        let frame = {
            let mut state = entry.state.lock().await;
            self.ensure_running()?;
            let mut candidate = state.chat.clone();
            let mut services = Vec::new();
            if candidate.rekey_due() {
                match candidate.start_rekey() {
                    Ok(Some(service)) => services.push(service),
                    Ok(None) => (),
                    Err(error) => {
                        self.commit_fatal_state(chat_id, &mut state, &candidate);
                        return Err(error);
                    }
                }
            }
            let prepared = candidate.prepare_service_action(action)?;
            let frame = prepared.frame.clone();
            services.push(prepared);
            self.stage_due_rekey(chat_id, &mut state, &mut candidate, &mut services)?;
            self.check_capacity(
                chat_id,
                &state,
                &BTreeSet::new(),
                services.len(),
                services.iter().map(|s| s.frame.len()).sum(),
            )?;
            self.retain_services(&mut state, services);
            state.chat = candidate;
            frame
        };
        self.transmit_pending(chat_id, &entry, Some(order)).await?;
        Ok(frame)
    }

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

    async fn entry(&self, chat_id: i64) -> Result<Arc<ChatEntry>, SecretChatError> {
        self.chats
            .lock()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or(SecretChatError::UnknownChat { chat_id })
    }

    fn emit(&self, event: SecretEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    fn ensure_running(&self) -> Result<(), SecretChatError> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(SecretChatError::ManagerStopping);
        }
        Ok(())
    }

    fn require_ready(&self, chat: &SecretChat) -> Result<(), SecretChatError> {
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

    fn check_capacity(
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

    fn retain_services(&self, state: &mut ManagedChat, services: Vec<PreparedServiceMessage>) {
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

    fn stage_due_rekey(
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

    fn commit_fatal_state(&self, chat_id: i64, state: &mut ManagedChat, candidate: &SecretChat) {
        if candidate.state() == ChatState::Closed && state.chat.state() != ChatState::Closed {
            state.chat.close();
            state.outbox.clear();
            self.emit(SecretEvent::ChatClosed {
                chat_id,
                reason: "authenticated protocol or key lifecycle violation".into(),
            });
        }
    }

    async fn transmit_pending(
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

fn text_identity(text: &str, ttl: i64) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"secret-chat-text\0");
    hash.update(ttl.to_le_bytes());
    hash.update(text.as_bytes());
    hash.finalize().into()
}
