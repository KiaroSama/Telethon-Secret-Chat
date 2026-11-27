use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::custody::{CustodyBackend, CustodyLimits, CustodyMode, CustodyRecord, CustodyStore};
use crate::errors::SecretChatError;
use crate::ingress::{
    DurableIngressController, DurableIngressRecord, IngressOrigin, IngressUpdate,
};
use crate::manager::{ManagerLimits, SecretTransport};
use crate::protocol::AcceptedMessage;
use crate::storage_backend::{PersistedChatRecord, SecretChatStore, corrupt};

use super::*;

pub struct BoundedSecretChatManager<T: SecretTransport, S: SecretChatStore> {
    persistent: PersistentSecretChatManager<T, S>,
    custody: Mutex<CustodyStore>,
    limits: CustodyLimits,
    max_age: Duration,
}

impl<T: SecretTransport, S: SecretChatStore> BoundedSecretChatManager<T, S> {
    pub fn new(
        persistent: PersistentSecretChatManager<T, S>,
        custody: CustodyStore,
        limits: CustodyLimits,
        max_age: Duration,
    ) -> Self {
        Self {
            persistent,
            custody: Mutex::new(custody),
            limits,
            max_age,
        }
    }

    pub async fn register_and_persist(&self, record: PersistedChatRecord) -> Result<()> {
        let mut store = self.custody.lock().await;
        let id = i64::from(record.chat_id);
        let owner = self.persistent.caller_auth_identity();
        match store.load_record(id, owner) {
            Ok(_) => (),
            Err(SecretChatError::UnknownChat { .. }) => store.save_record(&CustodyRecord::new(
                id,
                owner,
                CustodyMode::Bounded,
                CustodyBackend::EncryptedFile,
                self.limits,
            ))?,
            Err(err) => return Err(err),
        }
        self.persistent.register_and_persist(record).await
    }

    pub async fn custody_status(&self, id: i32) -> Result<CustodyRecord> {
        self.persistent.require_live(id).await?;
        self.load(&*self.custody.lock().await, id)
    }

    fn load(&self, store: &CustodyStore, id: i32) -> Result<CustodyRecord> {
        let record = store.load_record(i64::from(id), self.persistent.caller_auth_identity())?;
        if record.mode() != CustodyMode::Bounded
            || record.backend() != CustodyBackend::EncryptedFile
        {
            return Err(corrupt("custody record does not match bounded mode"));
        }
        Ok(record)
    }

    fn require_active(&self, record: &CustodyRecord) -> Result<()> {
        if record.is_paused() {
            return Err(SecretChatError::ChatNotReady {
                chat_id: record.chat_id(),
                state: "bounded custody is paused".into(),
            });
        }
        Ok(())
    }

    pub async fn send_text(&self, id: i32, text: &str) -> Result<Vec<u8>> {
        self.require_active(&self.custody_status(id).await?)?;
        self.persistent.send_text(id, text).await
    }

    pub async fn handle_incoming(&self, id: i32, frame: &[u8]) -> Result<Vec<AcceptedMessage>> {
        self.persistent.require_live(id).await?;
        {
            let mut store = self.custody.lock().await;
            let mut record = self.load(&store, id)?;
            self.require_active(&record)?;
            let retained = record.push_payload(frame);
            if retained.is_ok() || record.is_paused() {
                store.save_record(&record)?;
            }
            retained?;
        }
        self.persistent.handle_incoming(id, frame).await
    }

    pub async fn pause_chat(&self, id: i32) -> Result<()> {
        self.pause_for(id, "paused by caller").await
    }

    pub async fn note_missed_ingress(&self, id: i32) -> Result<()> {
        self.pause_for(id, MISSED_INGRESS).await
    }

    async fn pause_for(&self, id: i32, reason: &'static str) -> Result<()> {
        self.persistent.require_live(id).await?;
        let mut store = self.custody.lock().await;
        let mut record = self.load(&store, id)?;
        if record.pause_reason() != Some(MISSED_INGRESS) {
            record.pause(reason);
            store.save_record(&record)?;
        }
        Ok(())
    }

    pub async fn resume_chat(&self, id: i32) -> Result<()> {
        self.persistent.require_live(id).await?;
        let mut store = self.custody.lock().await;
        let mut record = self.load(&store, id)?;
        if record.pause_reason() == Some(MISSED_INGRESS) {
            return Err(SecretChatError::Protocol(MISSED_INGRESS));
        }
        record.resume();
        store.save_record(&record)
    }

    pub async fn sweep_custody(&self, now_unix: i64) -> Result<Vec<i32>> {
        let mut paused = Vec::new();
        for id in self.persistent.list_persisted_chats().await? {
            let chat = self.persistent.require_live(id).await?;
            let mut store = self.custody.lock().await;
            let mut record = self.load(&store, id)?;
            if record.is_paused() {
                continue;
            }
            let age =
                u64::try_from(now_unix.saturating_sub(chat.created_at_unix)).unwrap_or_default();
            let limits = record.limits();
            let reason = if record.retained_count() >= limits.max_retained_messages
                || record.retained_bytes() >= limits.max_stored_bytes
            {
                CAPACITY_PAUSE
            } else if Duration::from_secs(age) > self.max_age {
                MAX_AGE_PAUSE
            } else {
                continue;
            };
            record.pause(reason);
            store.save_record(&record)?;
            paused.push(id);
        }
        Ok(paused)
    }
}

pub struct TransientSecretChatManager<T: SecretTransport, S: SecretChatStore> {
    inner: SecretChatManager<T>,
    transport: Arc<T>,
    markers: S,
    active_chats: Mutex<BTreeSet<i32>>,
}

impl<T: SecretTransport, S: SecretChatStore> TransientSecretChatManager<T, S> {
    pub async fn open_after_restart(
        transport: T,
        markers: S,
        limits: ManagerLimits,
    ) -> Result<Self> {
        markers.load_all()?.validate(transport.auth_identity())?;
        let transport = Arc::new(transport);
        Ok(Self {
            inner: SecretChatManager::with_limits(transport.clone(), limits)?,
            transport,
            markers,
            active_chats: Mutex::new(BTreeSet::new()),
        })
    }

    pub async fn register_chat(&self, record: PersistedChatRecord) -> Result<()> {
        let mut active = self.active_chats.lock().await;
        let id = record.chat_id;
        if active.contains(&id) {
            return Err(SecretChatError::Protocol(
                "transient chat is already registered",
            ));
        }
        if self.markers.is_transient_suspended(id)? {
            return Err(SecretChatError::Protocol(SUSPENDED));
        }
        if record.tombstoned || record.protocol.is_some() || !record.pending_outbox.is_empty() {
            return Err(SecretChatError::Protocol(
                "transient mode requires a new chat",
            ));
        }
        let (chat, _) = restore(&record, self.transport.auth_identity())?;
        // Persist the marker before making the RAM-only chat usable.
        self.markers.mark_transient_suspended(id)?;
        self.inner.restore_chat(chat, Vec::new()).await?;
        active.insert(id);
        Ok(())
    }

    async fn active_ttl(&self, id: i32) -> Result<i32> {
        if self.active_chats.lock().await.contains(&id) {
            return self.inner.chat_ttl(i64::from(id)).await;
        }
        if self.markers.is_transient_suspended(id)? {
            return Err(SecretChatError::Protocol(SUSPENDED));
        }
        Err(unknown(id))
    }

    pub async fn send_text(&self, id: i32, text: &str) -> Result<Vec<u8>> {
        let ttl = self.active_ttl(id).await?;
        self.inner
            .send_text(i64::from(id), random_id()?, text, i64::from(ttl))
            .await
    }

    pub async fn handle_incoming(&self, id: i32, frame: &[u8]) -> Result<Vec<AcceptedMessage>> {
        self.active_ttl(id).await?;
        self.inner.receive_frames(i64::from(id), frame).await
    }

    pub async fn close_and_forget(&self, id: i32, delete_history: bool) -> Result<()> {
        let active = self.active_chats.lock().await.remove(&id);
        if !active && !self.markers.is_transient_suspended(id)? {
            return Err(unknown(id));
        }
        let result = if active {
            self.inner
                .close_chat(i64::from(id), delete_history, "closed by caller")
                .await
        } else {
            self.transport
                .discard_encryption(i64::from(id), delete_history)
                .await
        };
        result?;
        self.inner.remove_chat(i64::from(id)).await;
        self.markers.delete(id)
    }
}

/// Structured per-chat summary returned by `StrongSecretChatManager::replay_pending`.
#[derive(Debug, Default)]
pub struct ReplayOutcome {
    pub applied: Vec<(i32, i64, Vec<AcceptedMessage>)>,
    pub discarded_closed: Vec<(i32, usize)>,
    pub deferred_not_ready: Vec<(i32, i64)>,
    pub chat_errors: Vec<(i32, i64, SecretChatError)>,
}

pub struct StrongSecretChatManager<T: SecretTransport, S: SecretChatStore> {
    persistent: PersistentSecretChatManager<T, S>,
    ingress: Mutex<DurableIngressController>,
}

impl<T: SecretTransport, S: SecretChatStore> StrongSecretChatManager<T, S> {
    pub fn new(
        persistent: PersistentSecretChatManager<T, S>,
        ingress: DurableIngressController,
    ) -> Result<Self> {
        if persistent.caller_auth_identity() != ingress.owner_auth_id() {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress and chat stores belong to different authorizations".into(),
            });
        }
        Ok(Self {
            persistent,
            ingress: Mutex::new(ingress),
        })
    }

    pub fn stream_id(chat_id: i32) -> String {
        format!("secret-chat/{chat_id}")
    }

    pub async fn register_and_persist(&self, record: PersistedChatRecord) -> Result<()> {
        self.persistent.register_and_persist(record).await
    }

    pub async fn send_text(&self, id: i32, text: &str) -> Result<Vec<u8>> {
        self.persistent.send_text(id, text).await
    }

    /// Returning success authorizes the verified transport to acknowledge this update.
    pub async fn ingest_before_ack(&self, update: IngressUpdate) -> Result<DurableIngressRecord> {
        let id = update
            .associated_chat_id
            .and_then(|id| i32::try_from(id).ok())
            .filter(|id| *id > 0);
        let Some(id) = id.filter(|id| update.stream_id == Self::stream_id(*id)) else {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress must identify its own chat stream".into(),
            });
        };
        if update.provenance != IngressOrigin::VerifiedTransport || update.order <= 0 {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress requires verified provenance and positive order".into(),
            });
        }
        self.persistent.require_live(id).await?;
        self.ingress.lock().await.capture_and_commit(update)
    }

    pub async fn list_pending_deliveries(
        &self,
        id: i32,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<DurableIngressRecord>> {
        self.ingress
            .lock()
            .await
            .list_pending_page(&Self::stream_id(id), offset, limit)
    }

    pub async fn applied_cursor(&self, id: i32) -> Result<i64> {
        self.ingress
            .lock()
            .await
            .applied_cursor(&Self::stream_id(id))
    }

    pub async fn acknowledge_delivery(&self, id: i32, seq_no: i64) -> Result<bool> {
        let mut ingress = self.ingress.lock().await;
        let stream = Self::stream_id(id);
        let acknowledged = ingress.acknowledge_contiguous(&stream, seq_no)?;
        // Settle the durable cursor first; retries can finish receipt cleanup.
        let cursor = ingress.applied_cursor(&stream)?;
        self.persistent.settle_retained_deliveries(id, cursor)?;
        Ok(acknowledged)
    }

    /// Apply a durably captured frame for a single chat. Polling and replay never settle it.
    pub async fn apply_delivery(&self, id: i32, order: i64) -> Result<Vec<AcceptedMessage>> {
        self.persistent.require_live(id).await?;
        let stream = Self::stream_id(id);
        let record = {
            let mut ingress = self.ingress.lock().await;
            let cursor = ingress.applied_cursor(&stream)?;
            self.persistent.settle_retained_deliveries(id, cursor)?;
            let next = cursor.checked_add(1);
            if next != Some(order) {
                return Err(SecretChatError::Protocol(
                    "delivery is not next in contiguous order",
                ));
            }
            ingress
                .list_pending_page(&stream, 0, 1)?
                .into_iter()
                .next()
                .filter(|record| {
                    record.order == order && record.provenance == IngressOrigin::VerifiedTransport
                })
                .ok_or(SecretChatError::Protocol(
                    "next verified delivery is missing",
                ))?
        };
        self.persistent
            .handle_incoming_retained(id, order, &record.payload)
            .await
    }

    /// Close and tombstone a chat, discarding any stale pending ingress for its stream
    /// so subsequent startup replay or session checks never fail on closed-chat state.
    pub async fn close_and_tombstone(&self, id: i32, delete_history: bool) -> Result<()> {
        self.persistent
            .close_and_tombstone(id, delete_history)
            .await?;
        let stream = Self::stream_id(id);
        let _ = self.ingress.lock().await.discard_terminal_stream(&stream)?;
        Ok(())
    }

    /// Replay pending durable ingress across all chats on startup without letting a
    /// closed, tombstoned, unknown, or not-yet-ready chat abort startup or poison ingress.
    pub async fn replay_pending(&self) -> Result<ReplayOutcome> {
        let streams = self.ingress.lock().await.pending_streams()?;
        let mut outcome = ReplayOutcome::default();
        for stream in streams {
            let Some(id) = stream
                .strip_prefix("secret-chat/")
                .and_then(|raw| raw.parse::<i32>().ok())
                .filter(|id| *id > 0)
            else {
                continue;
            };
            match self.persistent.chat_state(id).await {
                None | Some(crate::protocol::ChatState::Closed) => {
                    let dropped = self.ingress.lock().await.discard_terminal_stream(&stream)?;
                    if dropped > 0 {
                        outcome.discarded_closed.push((id, dropped));
                    }
                }
                Some(
                    crate::protocol::ChatState::Requested | crate::protocol::ChatState::Offered,
                ) => {
                    let next = self.ingress.lock().await.applied_cursor(&stream)? + 1;
                    outcome.deferred_not_ready.push((id, next));
                }
                Some(crate::protocol::ChatState::Ready) => {
                    let cursor = self.ingress.lock().await.applied_cursor(&stream)?;
                    self.persistent.settle_retained_deliveries(id, cursor)?;
                    let Some(next) = cursor.checked_add(1) else {
                        continue;
                    };
                    let candidate = self
                        .ingress
                        .lock()
                        .await
                        .list_pending_page(&stream, 0, 1)?
                        .into_iter()
                        .next()
                        .filter(|rec| {
                            rec.order == next && rec.provenance == IngressOrigin::VerifiedTransport
                        });
                    if let Some(rec) = candidate {
                        match self
                            .persistent
                            .handle_incoming_retained(id, rec.order, &rec.payload)
                            .await
                        {
                            Ok(messages) => outcome.applied.push((id, rec.order, messages)),
                            Err(
                                SecretChatError::ChatClosed { .. }
                                | SecretChatError::UnknownChat { .. },
                            ) => {
                                let dropped =
                                    self.ingress.lock().await.discard_terminal_stream(&stream)?;
                                if dropped > 0 {
                                    outcome.discarded_closed.push((id, dropped));
                                }
                            }
                            Err(SecretChatError::ChatNotReady { .. }) => {
                                outcome.deferred_not_ready.push((id, rec.order));
                            }
                            Err(err) => outcome.chat_errors.push((id, rec.order, err)),
                        }
                    }
                }
            }
        }
        Ok(outcome)
    }
}
