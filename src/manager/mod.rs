mod delivery;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as EventMutex};

use serde_json::{Value, json};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::errors::SecretChatError;
use crate::protocol::{
    AcceptedMessage, ChatRole, MAX_ENCRYPTED_FRAME_LEN, SecretChat, encrypt_frame, outgoing_layer,
    wrap_frame,
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
        let identity = delivery::text_identity(text, ttl);

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
}
