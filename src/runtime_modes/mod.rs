//! Restartable, bounded, transient, and durable-ingress secret-chat managers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as RecordMutex, MutexGuard};

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

use crate::errors::SecretChatError;
use crate::manager::{
    BoxFuture, ChatCheckpoint, CoreSecretChatManager, EventRetention, ManagerLimits,
    PersistedOutboxItem, ReceivedBatch, SecretTransport, TextTtl,
};
use crate::protocol::{AcceptedMessage, ChatState, SecretChat};
use crate::storage_backend::{
    OutboxState, PendingDelivery, PersistedChatRecord, PersistedOutboxEntry, SavedAcceptedMessage,
    SecretChatStore, corrupt, unix_now,
};

type Result<T, E = SecretChatError> = std::result::Result<T, E>;

const TTL_ACTION: u32 = 0xa1733aec;
const READ_ACTION: u32 = 0x0c4f40be;
const SUSPENDED: &str = "transient chat permanently suspended after restart";
const MISSED_INGRESS: &str = "required ingress was missed while custody was paused";
const MAX_AGE_PAUSE: &str = "bounded custody age limit reached";
const CAPACITY_PAUSE: &str = "bounded custody capacity reached";

fn unknown(chat_id: i32) -> SecretChatError {
    SecretChatError::UnknownChat {
        chat_id: i64::from(chat_id),
    }
}

fn random_id() -> Result<i64> {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).map_err(|_| SecretChatError::EntropyUnavailable {
        reason: "OS randomness unavailable".into(),
    })?;
    Ok(i64::from_le_bytes(bytes))
}

fn counter(value: i64) -> Result<i32> {
    i32::try_from(value)
        .map_err(|_| corrupt("secret-chat sequence or layer is outside its stored range"))
}

fn restore(
    record: &PersistedChatRecord,
    owner: &str,
) -> Result<(SecretChat, Vec<PersistedOutboxItem>)> {
    record.validate()?;
    let mut chat = SecretChat::new(
        i64::from(record.chat_id),
        record.access_hash,
        record.peer_user_id,
        owner,
        record.role,
    );
    chat.adopt_key(&record.auth_key)?;
    if let Some(rt) = &record.protocol {
        let mut rt = rt.clone();
        rt.rekey_age_seconds = rt.rekey_age_seconds.saturating_add(
            u64::try_from(unix_now().saturating_sub(record.updated_at_unix)).unwrap_or_default(),
        );
        chat.import_runtime(&rt)?;
    } else {
        let mut rt = chat.export_runtime();
        rt.negotiated_layer = i64::from(record.layer);
        rt.ttl = record.ttl;
        rt.wrapper_layer = i64::from(record.layer);
        chat.import_runtime(&rt)?;
    }
    let outbox = record
        .pending_outbox
        .iter()
        .zip(&record.outbox)
        .map(|(item, meta)| PersistedOutboxItem {
            out_seq_no: i64::from(item.out_seq_no),
            random_id: item.random_id,
            identity: meta.identity,
            frame: item.payload.clone(),
            pending: meta.pending,
            service: meta.service,
            replay_requested: meta.replay_requested,
            acknowledged: meta.acknowledged,
            revision: meta.revision,
        })
        .collect();
    Ok((chat, outbox))
}

fn update_record(
    record: &mut PersistedChatRecord,
    chat: &SecretChat,
    items: Vec<PersistedOutboxItem>,
) -> Result<()> {
    if chat.state() == ChatState::Closed {
        record.tombstone();
        return Ok(());
    }
    let rt = chat.export_runtime();
    let active = rt
        .active_key
        .as_ref()
        .ok_or_else(|| corrupt("live secret chat has no active key"))?;
    let active_key = <[u8; 256]>::try_from(active.as_slice())
        .map_err(|_| corrupt("invalid active shared key size"))?;
    record.auth_key.zeroize();
    record.auth_key = active_key;
    record.previous_auth_key.zeroize();
    record.previous_auth_key = rt
        .previous_key
        .as_ref()
        .map(|key| <[u8; 256]>::try_from(key.as_slice()))
        .transpose()
        .map_err(|_| corrupt("invalid preceding shared key size"))?;
    record.previous_key_max_in_seq = rt
        .incoming_key_cutover
        .map(|seq| counter(seq.saturating_sub(1)))
        .transpose()?;
    record.rekey_state = rt.key_lifecycle;
    record.key_fingerprint = crate::protocol::key_fingerprint(&record.auth_key)?;
    record.state = rt.state;
    record.layer = counter(rt.negotiated_layer)?;
    record.ttl = chat.ttl();
    record.in_seq_no = counter(rt.in_seq_no)?;
    record.out_seq_no = counter(rt.out_seq_no)?;
    record.sent_since_rekey = rt.outgoing;
    record.received_since_rekey = rt.messages;
    let previous = std::mem::take(&mut record.pending_outbox);
    record.outbox.clear();
    for item in items {
        let seq = counter(item.out_seq_no)?;
        let attempts = previous
            .binary_search_by_key(&seq, |r| r.out_seq_no)
            .map_or(0, |i| previous[i].attempts);
        record.pending_outbox.push(PersistedOutboxEntry {
            random_id: item.random_id,
            out_seq_no: seq,
            payload: item.frame.clone(),
            attempts,
        });
        record.outbox.push(OutboxState {
            identity: item.identity,
            pending: item.pending,
            service: item.service,
            replay_requested: item.replay_requested,
            acknowledged: item.acknowledged,
            revision: item.revision,
        });
    }
    record.protocol = Some(rt);
    record.updated_at_unix = record.updated_at_unix.max(unix_now());
    Ok(())
}

struct CheckpointTransport<T: SecretTransport, S: SecretChatStore> {
    transport: T,
    store: S,
    records: RecordMutex<BTreeMap<i32, PersistedChatRecord>>,
    limits: ManagerLimits,
}

impl<T: SecretTransport, S: SecretChatStore> CheckpointTransport<T, S> {
    fn records(&self) -> Result<MutexGuard<'_, BTreeMap<i32, PersistedChatRecord>>> {
        self.records
            .lock()
            .map_err(|_| corrupt("checkpoint state lock was poisoned"))
    }

    fn checkpoint_attempt(&self, chat_id: i64, random: i64) -> Result<()> {
        let id = counter(chat_id)?;
        let mut records = self.records()?;
        let old = records.get(&id).ok_or_else(|| unknown(id))?;
        if old.tombstoned {
            return Err(SecretChatError::ChatClosed {
                chat_id,
                reason: "chat was tombstoned".into(),
            });
        }
        let mut next = old.clone();
        let entry = next
            .pending_outbox
            .iter_mut()
            .find(|entry| entry.random_id == random)
            .ok_or_else(|| corrupt("outbound frame was not retained for checkpointing"))?;
        entry.attempts = entry.attempts.saturating_add(1);
        self.store
            .save_checkpoint(Some(old.checkpoint), &mut next)?;
        records.insert(id, next);
        Ok(())
    }
}

impl<T: SecretTransport, S: SecretChatStore> ChatCheckpoint for CheckpointTransport<T, S> {
    fn commit(
        &self,
        chat: &SecretChat,
        outbox: Vec<PersistedOutboxItem>,
        received: Option<ReceivedBatch<'_>>,
    ) -> Result<()> {
        let id = counter(chat.chat_id())?;
        let mut records = self.records()?;
        let old = records.get(&id).ok_or_else(|| unknown(id))?;
        if old.tombstoned {
            return Err(SecretChatError::ChatClosed {
                chat_id: chat.chat_id(),
                reason: "chat was tombstoned".into(),
            });
        }
        let mut next = old.clone();
        update_record(&mut next, chat, outbox)?;
        if let Some(received) = received {
            if next.pending_deliveries.iter().any(|delivery| {
                delivery.frame_digest == received.frame_digest
                    || (received.ingress_order.is_some()
                        && delivery.ingress_order == received.ingress_order)
            }) {
                return Err(corrupt("an unsettled receive identity cannot be replaced"));
            }
            next.pending_deliveries.push(PendingDelivery {
                frame_digest: received.frame_digest,
                ingress_order: received.ingress_order,
                messages: received
                    .messages
                    .iter()
                    .map(SavedAcceptedMessage::capture)
                    .collect(),
            });
            self.check_delivery_capacity(&next)?;
        }
        self.store
            .save_checkpoint(Some(old.checkpoint), &mut next)?;
        records.insert(id, next);
        Ok(())
    }
}

impl<T: SecretTransport, S: SecretChatStore> CheckpointTransport<T, S> {
    fn check_delivery_capacity(&self, record: &PersistedChatRecord) -> Result<()> {
        let count = record
            .pending_deliveries
            .iter()
            .try_fold(0usize, |count, delivery| {
                count.checked_add(delivery.messages.len().max(1))
            });
        let bytes = Zeroizing::new(
            serde_json::to_vec(&record.pending_deliveries)
                .map_err(|_| corrupt("failed to encode pending received messages"))?,
        );
        if count.is_none_or(|count| count > self.limits.max_retained_messages)
            || (!record.pending_deliveries.is_empty()
                && bytes.len() > self.limits.max_retained_bytes)
        {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(i64::from(record.chat_id)),
                reason: "unsettled receive budget exhausted; settle pending deliveries".into(),
            });
        }
        Ok(())
    }
}

impl<T: SecretTransport, S: SecretChatStore> SecretTransport for CheckpointTransport<T, S> {
    fn auth_identity(&self) -> &str {
        self.transport.auth_identity()
    }

    fn send_encrypted<'a>(
        &'a self,
        id: i64,
        random: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.checkpoint_attempt(id, random)?;
            self.transport.send_encrypted(id, random, frame).await
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        id: i64,
        random: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.checkpoint_attempt(id, random)?;
            self.transport
                .send_encrypted_service(id, random, frame)
                .await
        })
    }

    fn discard_encryption(&self, id: i64, delete_history: bool) -> BoxFuture<'_, Result<()>> {
        self.transport.discard_encryption(id, delete_history)
    }
}

pub struct PersistentSecretChatManager<T: SecretTransport, S: SecretChatStore> {
    inner: Arc<CoreSecretChatManager<CheckpointTransport<T, S>>>,
    checkpoint: Arc<CheckpointTransport<T, S>>,
    registration: Mutex<()>,
}

impl<T: SecretTransport, S: SecretChatStore> PersistentSecretChatManager<T, S> {
    pub async fn open(transport: T, store: S, limits: ManagerLimits) -> Result<Self> {
        let snapshot = store.load_all()?;
        snapshot.validate(transport.auth_identity())?;
        let checkpoint = Arc::new(CheckpointTransport {
            transport,
            store,
            records: RecordMutex::new(snapshot.chats),
            limits,
        });
        let inner = Arc::new(CoreSecretChatManager::with_limits_and_event_retention(
            checkpoint.clone(),
            limits,
            EventRetention::Discard,
        )?);
        let records: Vec<_> = checkpoint.records()?.values().cloned().collect();
        for record in records.iter().filter(|r| !r.tombstoned) {
            checkpoint.check_delivery_capacity(record)?;
            let (chat, outbox) = restore(record, checkpoint.auth_identity())?;
            inner.restore_chat(chat, outbox).await?;
        }
        Ok(Self {
            inner,
            checkpoint,
            registration: Mutex::new(()),
        })
    }

    pub fn caller_auth_identity(&self) -> &str {
        self.checkpoint.auth_identity()
    }

    pub async fn register_and_persist(&self, mut record: PersistedChatRecord) -> Result<()> {
        let _registration = self.registration.lock().await;
        if record.tombstoned || self.checkpoint.records()?.contains_key(&record.chat_id) {
            return Err(corrupt("chat identity is closed or already registered"));
        }
        let (chat, outbox) = restore(&record, self.caller_auth_identity())?;
        let id = record.chat_id;
        self.inner.restore_chat(chat, outbox).await?;
        let result = async {
            let (chat, outbox) = self.inner.snapshot_chat(i64::from(id)).await?;
            update_record(&mut record, &chat, outbox)?;
            self.checkpoint.check_delivery_capacity(&record)?;
            let mut records = self.checkpoint.records()?;
            self.checkpoint.store.save_checkpoint(None, &mut record)?;
            records.insert(id, record.clone());
            Ok(())
        }
        .await;
        if let Err(err) = result {
            self.inner.remove_chat(i64::from(id)).await;
            return Err(err);
        }
        Ok(())
    }

    async fn require_live(&self, id: i32) -> Result<PersistedChatRecord> {
        let record = self.snapshot_chat(id).await?;
        if record.tombstoned {
            return Err(SecretChatError::ChatClosed {
                chat_id: i64::from(id),
                reason: "chat was tombstoned".into(),
            });
        }
        Ok(record)
    }

    pub async fn send_text(&self, id: i32, text: &str) -> Result<Vec<u8>> {
        self.send_text_with_id(id, random_id()?, text).await
    }

    pub async fn send_text_with_id(&self, id: i32, random_id: i64, text: &str) -> Result<Vec<u8>> {
        self.require_live(id).await?;
        self.inner
            .send_text_checkpointed(
                i64::from(id),
                random_id,
                text,
                TextTtl::Conversation,
                Some(self.checkpoint.as_ref()),
            )
            .await
    }

    pub async fn retry_pending(&self, id: i32) -> Result<usize> {
        self.require_live(id).await?;
        self.inner
            .retry_pending_checkpointed(i64::from(id), Some(self.checkpoint.as_ref()))
            .await
    }

    pub async fn configure_chat_dh(&self, id: i32, g: i64, p: &[u8]) -> Result<()> {
        self.require_live(id).await?;
        self.inner
            .configure_chat_dh_checkpointed(i64::from(id), g, p, Some(self.checkpoint.as_ref()))
            .await
    }

    pub async fn start_rekey(&self, id: i32) -> Result<bool> {
        self.require_live(id).await?;
        self.inner
            .start_rekey_checkpointed(i64::from(id), Some(self.checkpoint.as_ref()))
            .await
    }

    /// Retry the same input after an interrupted call to recover its accepted batch.
    /// Response RPC failure does not hide accepted messages; responses stay retryable.
    /// A successful return settles this receipt; strong mode waits for consumer ACK.
    pub async fn handle_incoming(&self, id: i32, frame: &[u8]) -> Result<Vec<AcceptedMessage>> {
        self.require_live(id).await?;
        let digest = Sha256::digest(frame).into();
        if let Some(messages) = self.received_delivery(id, digest, None, true)? {
            return Ok(messages);
        }
        let result = self
            .inner
            .receive_frames_checkpointed(i64::from(id), frame, None, Some(self.checkpoint.as_ref()))
            .await;
        if let Some(messages) = self.received_delivery(id, digest, None, true)? {
            return Ok(messages);
        }
        // Another concurrent receiver may already have returned this committed batch.
        result.map(|_| Vec::new())
    }

    pub(crate) async fn handle_incoming_retained(
        &self,
        id: i32,
        order: i64,
        frame: &[u8],
    ) -> Result<Vec<AcceptedMessage>> {
        self.require_live(id).await?;
        if order <= 0 {
            return Err(corrupt("retained delivery order must be positive"));
        }
        let digest = Sha256::digest(frame).into();
        if let Some(messages) = self.received_delivery(id, digest, Some(order), false)? {
            return Ok(messages);
        }
        let result = self
            .inner
            .receive_frames_checkpointed(
                i64::from(id),
                frame,
                Some(order),
                Some(self.checkpoint.as_ref()),
            )
            .await;
        match self.received_delivery(id, digest, Some(order), false)? {
            Some(messages) => Ok(messages),
            None => result,
        }
    }

    fn received_delivery(
        &self,
        id: i32,
        digest: [u8; 32],
        order: Option<i64>,
        settle: bool,
    ) -> Result<Option<Vec<AcceptedMessage>>> {
        let mut records = self.checkpoint.records()?;
        let old = records.get(&id).ok_or_else(|| unknown(id))?;
        let Some(delivery) = old
            .pending_deliveries
            .iter()
            .find(|delivery| delivery.frame_digest == digest)
        else {
            return Ok(None);
        };
        if delivery.ingress_order != order {
            return Err(corrupt(
                "receive identity belongs to another delivery order",
            ));
        }
        let messages = delivery
            .messages
            .iter()
            .map(SavedAcceptedMessage::message)
            .collect();
        if settle {
            let mut next = old.clone();
            next.pending_deliveries
                .retain(|delivery| delivery.frame_digest != digest);
            self.checkpoint
                .store
                .save_checkpoint(Some(old.checkpoint), &mut next)?;
            records.insert(id, next);
        }
        Ok(Some(messages))
    }

    /// Remove only receipts covered by an already committed contiguous ingress cursor.
    pub(crate) fn settle_retained_deliveries(&self, id: i32, through: i64) -> Result<()> {
        let mut records = self.checkpoint.records()?;
        let old = records.get(&id).ok_or_else(|| unknown(id))?;
        if !old
            .pending_deliveries
            .iter()
            .any(|delivery| delivery.ingress_order.is_some_and(|order| order <= through))
        {
            return Ok(());
        }
        let mut next = old.clone();
        next.pending_deliveries
            .retain(|delivery| delivery.ingress_order.is_none_or(|order| order > through));
        self.checkpoint
            .store
            .save_checkpoint(Some(old.checkpoint), &mut next)?;
        records.insert(id, next);
        Ok(())
    }

    pub async fn set_ttl(&self, id: i32, ttl: i32) -> Result<Vec<u8>> {
        self.service(
            id,
            json!({"id": TTL_ACTION, "fields": {"ttl_seconds": ttl}}),
        )
        .await
    }

    pub async fn mark_read(&self, id: i32, random_ids: &[i64]) -> Result<Vec<u8>> {
        if random_ids.len() > 4096 {
            return Err(SecretChatError::Protocol("too many read acknowledgments"));
        }
        self.service(
            id,
            json!({"id": READ_ACTION, "fields": {"random_ids": random_ids}}),
        )
        .await
    }

    async fn service(&self, id: i32, action: serde_json::Value) -> Result<Vec<u8>> {
        self.require_live(id).await?;
        self.inner
            .send_service_action_checkpointed(
                i64::from(id),
                &action,
                Some(self.checkpoint.as_ref()),
            )
            .await
    }

    pub async fn close_and_tombstone(&self, id: i32, delete_history: bool) -> Result<()> {
        self.require_live(id).await?;
        self.inner
            .close_chat_checkpointed(
                i64::from(id),
                delete_history,
                "closed by caller",
                Some(self.checkpoint.as_ref()),
            )
            .await
    }

    pub async fn list_persisted_chats(&self) -> Result<Vec<i32>> {
        Ok(self
            .checkpoint
            .records()?
            .values()
            .filter(|r| !r.tombstoned)
            .map(|r| r.chat_id)
            .collect())
    }

    pub async fn snapshot_chat(&self, id: i32) -> Result<PersistedChatRecord> {
        self.checkpoint
            .records()?
            .get(&id)
            .cloned()
            .ok_or_else(|| unknown(id))
    }

    pub(crate) async fn chat_state(&self, id: i32) -> Option<ChatState> {
        self.checkpoint
            .records()
            .ok()?
            .get(&id)
            .filter(|r| !r.tombstoned)
            .map(|r| r.state)
    }
}

mod modes;
pub use modes::{
    BoundedSecretChatManager, ReplayOutcome, StrongSecretChatManager, TransientSecretChatManager,
};

#[cfg(test)]
mod tests;
