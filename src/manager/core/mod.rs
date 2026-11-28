mod delivery;
mod text;

pub(crate) use text::TextTtl;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as EventMutex};

use serde_json::Value;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::errors::SecretChatError;
use crate::protocol::{AcceptedMessage, ChatRole, MAX_ENCRYPTED_FRAME_LEN, SecretChat};

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

/// Finite retained-outbox budgets. Persistent modes also apply these bounds to
/// accepted batches awaiting delivery. Exhaustion refuses work without evicting data.
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

#[derive(Clone, serde::Serialize, serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedOutboxItem {
    pub out_seq_no: i64,
    pub random_id: i64,
    pub identity: [u8; 32],
    pub frame: Vec<u8>,
    pub pending: bool,
    pub service: bool,
    pub replay_requested: bool,
    pub acknowledged: bool,
    pub revision: u64,
}

pub(crate) struct ReceivedBatch<'a> {
    pub frame_digest: [u8; 32],
    pub ingress_order: Option<i64>,
    pub messages: &'a [AcceptedMessage],
}

/// Synchronous persistence runs under the chat state lock, before publication.
pub(crate) trait ChatCheckpoint: Send + Sync {
    fn commit(
        &self,
        chat: &SecretChat,
        outbox: Vec<PersistedOutboxItem>,
        received: Option<ReceivedBatch<'_>>,
    ) -> Result<(), SecretChatError>;
}

#[derive(Clone, Copy)]
pub(crate) enum EventRetention {
    Queue,
    Discard,
}

#[derive(Clone)]
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

#[derive(Clone)]
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
pub struct CoreSecretChatManager<T: SecretTransport> {
    transport: Arc<T>,
    chats: Mutex<BTreeMap<i64, Arc<ChatEntry>>>,
    events: EventMutex<Vec<SecretEvent>>,
    event_retention: EventRetention,
    stopping: AtomicBool,
    limits: ManagerLimits,
}

impl<T: SecretTransport> CoreSecretChatManager<T> {
    pub fn new(transport: Arc<T>) -> Self {
        Self {
            transport,
            chats: Mutex::new(BTreeMap::new()),
            events: EventMutex::new(Vec::new()),
            event_retention: EventRetention::Queue,
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

    pub(crate) fn with_limits_and_event_retention(
        transport: Arc<T>,
        limits: ManagerLimits,
        event_retention: EventRetention,
    ) -> Result<Self, SecretChatError> {
        let mut manager = Self::with_limits(transport, limits)?;
        manager.event_retention = event_retention;
        Ok(manager)
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

    /// Retry unsettled transmissions; successful RPCs remain retained until peer acknowledgement.
    pub async fn retry_pending(&self, chat_id: i64) -> Result<usize, SecretChatError> {
        self.retry_pending_checkpointed(chat_id, None).await
    }

    pub(crate) async fn retry_pending_checkpointed(
        &self,
        chat_id: i64,
        checkpoint: Option<&dyn ChatCheckpoint>,
    ) -> Result<usize, SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.transmit_pending(chat_id, &entry, Some(order), checkpoint)
            .await
    }

    /// Supply the validated original DH parameters before the automatic rekey threshold.
    pub async fn configure_chat_dh(
        &self,
        chat_id: i64,
        g: i64,
        p: &[u8],
    ) -> Result<(), SecretChatError> {
        self.configure_chat_dh_checkpointed(chat_id, g, p, None)
            .await
    }

    pub(crate) async fn configure_chat_dh_checkpointed(
        &self,
        chat_id: i64,
        g: i64,
        p: &[u8],
        checkpoint: Option<&dyn ChatCheckpoint>,
    ) -> Result<(), SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let mut state = entry.state.lock().await;
        self.ensure_running()?;
        let mut next = state.clone();
        next.chat.configure_dh(g, p)?;
        self.commit_candidate(&mut state, next, checkpoint, None)
    }

    /// Begin a key exchange, retaining its exact service frame before any network await.
    pub async fn start_rekey(&self, chat_id: i64) -> Result<bool, SecretChatError> {
        self.start_rekey_checkpointed(chat_id, None).await
    }

    pub(crate) async fn start_rekey_checkpointed(
        &self,
        chat_id: i64,
        checkpoint: Option<&dyn ChatCheckpoint>,
    ) -> Result<bool, SecretChatError> {
        self.ensure_running()?;
        let entry = self.entry(chat_id).await?;
        let order = entry.outbound.lock().await;
        self.transmit_pending(chat_id, &entry, None, checkpoint)
            .await?;
        let started = {
            let mut state = entry.state.lock().await;
            self.ensure_running()?;
            let mut candidate = state.chat.clone();
            let prepared = match candidate.start_rekey() {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.commit_fatal_state(chat_id, &mut state, &candidate, checkpoint)?;
                    return Err(error);
                }
            };
            if let Some(prepared) = prepared {
                self.check_capacity(chat_id, &state, &BTreeSet::new(), 1, prepared.frame.len())?;
                let mut next = state.clone();
                self.retain_services(&mut next, vec![prepared]);
                next.chat = candidate;
                self.commit_candidate(&mut state, next, checkpoint, None)?;
                true
            } else {
                false
            }
        };
        self.transmit_pending(chat_id, &entry, Some(order), checkpoint)
            .await?;
        Ok(started)
    }

    /// Send a conversation action. Rekey and recovery actions are owned by the state machine.
    /// TTL, deletion, reading and archive effects remain the application's responsibility.
    pub async fn send_service_action(
        &self,
        chat_id: i64,
        action: &Value,
    ) -> Result<Vec<u8>, SecretChatError> {
        self.send_service_action_checkpointed(chat_id, action, None)
            .await
    }

    pub(crate) async fn send_service_action_checkpointed(
        &self,
        chat_id: i64,
        action: &Value,
        checkpoint: Option<&dyn ChatCheckpoint>,
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
        self.transmit_pending(chat_id, &entry, None, checkpoint)
            .await?;
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
                        self.commit_fatal_state(chat_id, &mut state, &candidate, checkpoint)?;
                        return Err(error);
                    }
                }
            }
            let outgoing_ttl = (action["id"] == 0xa173_3aec_u32)
                .then(|| action["fields"]["ttl_seconds"].as_i64())
                .flatten()
                .and_then(|v| i32::try_from(v).ok())
                .filter(|v| *v >= 0);
            let prepared = candidate.prepare_service_action(action)?;
            let frame = prepared.frame.clone();
            services.push(prepared);
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
                services.len(),
                services.iter().map(|s| s.frame.len()).sum(),
            )?;
            if let Some(ttl) = outgoing_ttl {
                candidate.set_persisted_ttl(ttl);
            }
            let mut next = state.clone();
            self.retain_services(&mut next, services);
            next.chat = candidate;
            self.commit_candidate(&mut state, next, checkpoint, None)?;
            frame
        };
        self.transmit_pending(chat_id, &entry, Some(order), checkpoint)
            .await?;
        Ok(frame)
    }

    pub(crate) async fn restore_chat(
        &self,
        chat: SecretChat,
        outbox_items: Vec<PersistedOutboxItem>,
    ) -> Result<(), SecretChatError> {
        let chat_id = chat.chat_id();
        if outbox_items.len() > self.limits.max_retained_messages {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(chat_id),
                reason: "persisted outbox exceeds retained message limit".into(),
            });
        }
        let mut total_bytes = 0usize;
        let mut seen_random_ids = BTreeSet::new();
        let mut outbox = BTreeMap::new();
        for item in outbox_items {
            if item.out_seq_no < 0 || item.out_seq_no >= chat.out_seq_no() {
                return Err(SecretChatError::StoreCorrupt {
                    chat_id: Some(chat_id),
                    reason: "persisted outbox sequence number is out of range".into(),
                });
            }
            if item.frame.len() > MAX_ENCRYPTED_FRAME_LEN {
                return Err(SecretChatError::StoreCorrupt {
                    chat_id: Some(chat_id),
                    reason: "persisted outbox frame exceeds maximum length".into(),
                });
            }
            total_bytes = total_bytes
                .checked_add(item.frame.len())
                .filter(|&sum| sum <= self.limits.max_retained_bytes)
                .ok_or_else(|| SecretChatError::StoreCorrupt {
                    chat_id: Some(chat_id),
                    reason: "persisted outbox exceeds retained byte limit".into(),
                })?;
            if !seen_random_ids.insert(item.random_id) {
                return Err(SecretChatError::StoreCorrupt {
                    chat_id: Some(chat_id),
                    reason: "duplicate random_id in persisted outbox".into(),
                });
            }
            if outbox
                .insert(
                    item.out_seq_no,
                    OutboundMessage {
                        random_id: item.random_id,
                        identity: item.identity,
                        frame: Zeroizing::new(item.frame.clone()),
                        pending: item.pending,
                        service: item.service,
                        replay_requested: item.replay_requested,
                        acknowledged: item.acknowledged,
                        revision: item.revision,
                    },
                )
                .is_some()
            {
                return Err(SecretChatError::StoreCorrupt {
                    chat_id: Some(chat_id),
                    reason: "duplicate sequence number in persisted outbox".into(),
                });
            }
        }
        let mut chats = self.chats.lock().await;
        chats.insert(
            chat_id,
            Arc::new(ChatEntry {
                state: Mutex::new(ManagedChat { chat, outbox }),
                outbound: Mutex::new(()),
            }),
        );
        Ok(())
    }

    pub(crate) async fn snapshot_chat(
        &self,
        chat_id: i64,
    ) -> Result<(SecretChat, Vec<PersistedOutboxItem>), SecretChatError> {
        let entry = self.entry(chat_id).await?;
        let state = entry.state.lock().await;
        Ok((state.chat.clone(), Self::snapshot_outbox(&state)))
    }

    fn snapshot_outbox(state: &ManagedChat) -> Vec<PersistedOutboxItem> {
        state
            .outbox
            .iter()
            .map(|(seq, msg)| PersistedOutboxItem {
                out_seq_no: *seq,
                random_id: msg.random_id,
                identity: msg.identity,
                frame: msg.frame.to_vec(),
                pending: msg.pending,
                service: msg.service,
                replay_requested: msg.replay_requested,
                acknowledged: msg.acknowledged,
                revision: msg.revision,
            })
            .collect()
    }

    fn commit_candidate(
        &self,
        state: &mut ManagedChat,
        next: ManagedChat,
        checkpoint: Option<&dyn ChatCheckpoint>,
        received: Option<ReceivedBatch<'_>>,
    ) -> Result<(), SecretChatError> {
        if let Some(checkpoint) = checkpoint {
            checkpoint.commit(&next.chat, Self::snapshot_outbox(&next), received)?;
        }
        *state = next;
        Ok(())
    }

    pub(crate) async fn remove_chat(&self, chat_id: i64) -> bool {
        self.chats.lock().await.remove(&chat_id).is_some()
    }
}
