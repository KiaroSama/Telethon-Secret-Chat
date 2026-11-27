use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use crate::errors::SecretChatError;

use super::*;

/// Owner-bound durable capture with deduplication and sorted pending snapshots.
///
/// This container does not attach to a transport, establish a verified baseline,
/// hold a protocol-applied cursor, or block delivery across missing stream orders.
/// The caller coordinates those boundaries and settles consumer effects explicitly.
pub struct DurableIngressController {
    owner_auth_id: String,
    store: EncryptedIngressStore,
    limits: IngressLimits,
    seen_orders: BTreeMap<String, BTreeSet<i64>>,
    pending_by_stream: BTreeMap<String, BTreeMap<i64, DurableIngressRecord>>,
}

impl std::fmt::Debug for DurableIngressController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableIngressController")
            .field("streams", &self.seen_orders.len())
            .field(
                "pending_records",
                &self
                    .pending_by_stream
                    .values()
                    .map(BTreeMap::len)
                    .sum::<usize>(),
            )
            .finish_non_exhaustive()
    }
}

impl DurableIngressController {
    /// Create fresh custody. The first write refuses an existing store; use
    /// `restore` to adopt and validate an existing snapshot.
    pub fn new(
        owner_auth_id: impl Into<String>,
        store: EncryptedIngressStore,
        limits: IngressLimits,
    ) -> Self {
        Self {
            owner_auth_id: owner_auth_id.into(),
            store,
            limits,
            seen_orders: BTreeMap::new(),
            pending_by_stream: BTreeMap::new(),
        }
    }

    pub fn restore(
        owner_auth_id: impl Into<String>,
        mut store: EncryptedIngressStore,
        limits: IngressLimits,
    ) -> Result<Self, SecretChatError> {
        let expected_owner = owner_auth_id.into();
        let persisted = store.load()?;
        let mut controller = Self::new(expected_owner.clone(), store, limits);
        controller.validate_configuration()?;
        if let Some(state) = persisted {
            if state.owner_auth_id != expected_owner {
                return Err(SecretChatError::IngressRefused {
                    reason: "durable ingress store belongs to a different authorization".into(),
                });
            }
            controller.validate_state(&state)?;
            controller.seen_orders = state.seen_orders;
            controller.pending_by_stream = state
                .pending_by_stream
                .into_iter()
                .map(|(stream, records)| {
                    let marked = records
                        .into_iter()
                        .map(|(ord, mut rec)| {
                            rec.status = CommitReplayStatus::Replayed;
                            (ord, rec)
                        })
                        .collect();
                    (stream, marked)
                })
                .collect();
        }
        Ok(controller)
    }

    pub fn owner_auth_id(&self) -> &str {
        &self.owner_auth_id
    }

    pub fn capture_and_commit(
        &mut self,
        update: IngressUpdate,
    ) -> Result<DurableIngressRecord, SecretChatError> {
        self.validate_update(&update)?;

        if self.is_duplicate(&update) {
            if let Some(original) = self
                .pending_by_stream
                .get(&update.stream_id)
                .and_then(|pending| pending.get(&update.order))
                && (original.payload != update.payload
                    || original.associated_chat_id != update.associated_chat_id)
            {
                return Err(EncryptedIngressStore::refused(
                    "duplicate ingress identity has conflicting content",
                ));
            }
            self.store.ensure_current()?;
            return Ok(DurableIngressRecord {
                stream_id: update.stream_id,
                order: update.order,
                provenance: update.provenance,
                validated: true,
                status: CommitReplayStatus::DuplicateIgnored,
                associated_chat_id: update.associated_chat_id,
                payload: update.payload,
            });
        }
        self.validate_capacity(&update)?;

        let record = DurableIngressRecord {
            stream_id: update.stream_id.clone(),
            order: update.order,
            provenance: update.provenance,
            validated: true,
            status: CommitReplayStatus::Committed,
            associated_chat_id: update.associated_chat_id,
            payload: update.payload,
        };

        // Stage candidate state, persist atomically FIRST, and adopt in memory ONLY on commit success.
        let mut next_seen = self.seen_orders.clone();
        next_seen
            .entry(record.stream_id.clone())
            .or_default()
            .insert(record.order);
        let mut next_pending = self.pending_by_stream.clone();
        next_pending
            .entry(record.stream_id.clone())
            .or_default()
            .insert(record.order, record.clone());

        let snapshot = PersistedIngressState {
            owner_auth_id: self.owner_auth_id.clone(),
            seen_orders: next_seen,
            pending_by_stream: next_pending,
        };
        self.store.save(&snapshot)?;

        self.seen_orders = snapshot.seen_orders;
        self.pending_by_stream = snapshot.pending_by_stream;
        Ok(record)
    }

    pub async fn capture_with_barrier<F, Fut>(
        &mut self,
        update: IngressUpdate,
        barrier: F,
    ) -> Result<DurableIngressRecord, SecretChatError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), SecretChatError>>,
    {
        self.validate_update(&update)?;
        if !self.is_duplicate(&update) {
            self.validate_capacity(&update)?;
        }
        barrier().await?;
        self.capture_and_commit(update)
    }

    /// Return detached pending records without acknowledging them.
    /// Commit the consumer's result before calling `acknowledge` for its stable identity.
    pub fn pending_in_order(&self, stream_id: &str) -> Vec<DurableIngressRecord> {
        self.pending_by_stream
            .get(stream_id)
            .map(|records| records.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Explicitly release one consumer-settled obligation after durable commit.
    /// Returns `false` for an already acknowledged identity; unknown identities refuse.
    pub fn acknowledge(&mut self, stream_id: &str, order: i64) -> Result<bool, SecretChatError> {
        self.validate_configuration()?;
        if !self
            .seen_orders
            .get(stream_id)
            .is_some_and(|orders| orders.contains(&order))
        {
            return Err(EncryptedIngressStore::refused(
                "cannot acknowledge an unknown ingress identity",
            ));
        }
        if !self
            .pending_by_stream
            .get(stream_id)
            .is_some_and(|records| records.contains_key(&order))
        {
            self.store.ensure_current()?;
            return Ok(false);
        }
        let mut next_pending = self.pending_by_stream.clone();
        if let Some(records) = next_pending.get_mut(stream_id) {
            records.remove(&order);
            if records.is_empty() {
                next_pending.remove(stream_id);
            }
        }
        let snapshot = PersistedIngressState {
            owner_auth_id: self.owner_auth_id.clone(),
            seen_orders: self.seen_orders.clone(),
            pending_by_stream: next_pending,
        };
        self.store.save(&snapshot)?;
        self.pending_by_stream = snapshot.pending_by_stream;
        Ok(true)
    }

    /// Return the highest contiguous settled 1-based order for `stream_id`.
    /// Calls `ensure_current()` first so stale snapshots fail closed.
    pub fn applied_cursor(&mut self, stream_id: &str) -> Result<i64, SecretChatError> {
        self.store.ensure_current()?;
        let seen = match self.seen_orders.get(stream_id) {
            Some(orders) => orders,
            None => return Ok(0),
        };
        let pending = self.pending_by_stream.get(stream_id);
        let mut cursor = 0i64;
        while let Some(next) = cursor.checked_add(1) {
            if seen.contains(&next) && !pending.is_some_and(|p| p.contains_key(&next)) {
                cursor = next;
            } else {
                break;
            }
        }
        Ok(cursor)
    }

    /// Acknowledge `order` only if it is the exact next contiguous 1-based order (`applied_cursor + 1`).
    pub fn acknowledge_contiguous(
        &mut self,
        stream_id: &str,
        order: i64,
    ) -> Result<bool, SecretChatError> {
        if order <= 0 {
            return Err(EncryptedIngressStore::refused(
                "contiguous ingress order must be positive",
            ));
        }
        let cursor = self.applied_cursor(stream_id)?;
        if order <= cursor {
            return Ok(false);
        }
        let expected = cursor
            .checked_add(1)
            .ok_or_else(|| EncryptedIngressStore::refused("contiguous ingress cursor overflow"))?;
        if order != expected {
            return Err(EncryptedIngressStore::refused(
                "cannot acknowledge non-contiguous ingress order ahead of applied cursor",
            ));
        }
        let is_pending = self
            .pending_by_stream
            .get(stream_id)
            .is_some_and(|p| p.contains_key(&order));
        if !is_pending {
            return Ok(false);
        }
        self.acknowledge(stream_id, order)
    }

    /// Poll a bounded page of pending records (`limit` in 1..=100, `offset` in 0..=4096) without acking.
    pub fn list_pending_page(
        &mut self,
        stream_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<DurableIngressRecord>, SecretChatError> {
        if offset > 4096 || !(1..=100).contains(&limit) {
            return Err(EncryptedIngressStore::refused(
                "pending delivery page parameters out of bounds",
            ));
        }
        self.store.ensure_current()?;
        Ok(self
            .pending_by_stream
            .get(stream_id)
            .map(|records| records.values().skip(offset).take(limit).cloned().collect())
            .unwrap_or_default())
    }

    /// Return all stream IDs that currently hold at least one pending record.
    pub fn pending_streams(&mut self) -> Result<Vec<String>, SecretChatError> {
        self.store.ensure_current()?;
        Ok(self
            .pending_by_stream
            .iter()
            .filter(|(_, records)| !records.is_empty())
            .map(|(stream, _)| stream.clone())
            .collect())
    }

    /// Discard pending records for a permanently closed, tombstoned, or unknown chat stream
    /// while preserving `seen_orders` so duplicate frames stay ignored.
    pub fn discard_terminal_stream(&mut self, stream_id: &str) -> Result<usize, SecretChatError> {
        self.validate_configuration()?;
        self.store.ensure_current()?;
        let count = self
            .pending_by_stream
            .get(stream_id)
            .map(BTreeMap::len)
            .unwrap_or(0);
        if count == 0 {
            return Ok(0);
        }
        let mut next_pending = self.pending_by_stream.clone();
        next_pending.remove(stream_id);
        let snapshot = PersistedIngressState {
            owner_auth_id: self.owner_auth_id.clone(),
            seen_orders: self.seen_orders.clone(),
            pending_by_stream: next_pending,
        };
        self.store.save(&snapshot)?;
        self.pending_by_stream = snapshot.pending_by_stream;
        Ok(count)
    }

    /// Atomically acknowledge and return the entire currently pending stream.
    /// For consumer crash recovery, prefer `pending_in_order` followed by `acknowledge`.
    /// Even an empty result requires the durable snapshot to remain current.
    pub fn drain_in_order(
        &mut self,
        stream_id: &str,
    ) -> Result<Vec<DurableIngressRecord>, SecretChatError> {
        self.validate_configuration()?;
        let drained = self.pending_in_order(stream_id);
        if drained.is_empty() {
            self.store.ensure_current()?;
            return Ok(drained);
        }
        let mut next_pending = self.pending_by_stream.clone();
        next_pending.remove(stream_id);
        let snapshot = PersistedIngressState {
            owner_auth_id: self.owner_auth_id.clone(),
            seen_orders: self.seen_orders.clone(),
            pending_by_stream: next_pending,
        };
        self.store.save(&snapshot)?;
        self.pending_by_stream = snapshot.pending_by_stream;
        Ok(drained)
    }

    fn validate_configuration(&self) -> Result<(), SecretChatError> {
        if self.owner_auth_id.trim().is_empty()
            || self.owner_auth_id.len() > MAX_ID_BYTES
            || self.limits.max_pending_records == 0
            || self.limits.max_pending_records > 4096
            || self.limits.max_payload_bytes == 0
            || self.limits.max_payload_bytes > 1024 * 1024
        {
            return Err(EncryptedIngressStore::refused(
                "ingress authorization identity or configured limits are invalid",
            ));
        }
        Ok(())
    }

    fn is_duplicate(&self, update: &IngressUpdate) -> bool {
        self.seen_orders
            .get(&update.stream_id)
            .is_some_and(|orders| orders.contains(&update.order))
    }

    fn validate_capacity(&self, update: &IngressUpdate) -> Result<(), SecretChatError> {
        let total_pending: usize = self.pending_by_stream.values().map(BTreeMap::len).sum();
        if total_pending >= self.limits.max_pending_records {
            return Err(EncryptedIngressStore::refused(
                "durable ingress pending queue limit reached",
            ));
        }
        let total_seen: usize = self.seen_orders.values().map(BTreeSet::len).sum();
        if total_seen >= MAX_TRACKED_ORDERS
            || (!self.seen_orders.contains_key(&update.stream_id)
                && self.seen_orders.len() >= MAX_STREAMS)
        {
            return Err(EncryptedIngressStore::refused(
                "durable ingress replay history budget reached",
            ));
        }
        let payload_bytes: usize = self
            .pending_by_stream
            .values()
            .flat_map(BTreeMap::values)
            .map(|record| record.payload.len())
            .sum();
        if update.payload.len() > MAX_PENDING_PAYLOAD_BYTES.saturating_sub(payload_bytes) {
            return Err(EncryptedIngressStore::refused(
                "durable ingress aggregate payload budget reached",
            ));
        }
        Ok(())
    }

    fn validate_update(&self, update: &IngressUpdate) -> Result<(), SecretChatError> {
        self.validate_configuration()?;
        if update.stream_id.trim().is_empty() || update.stream_id.len() > MAX_ID_BYTES {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress stream identity cannot be empty".into(),
            });
        }
        if update.order < 0 {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress order must be non-negative".into(),
            });
        }
        if update.payload.len() > self.limits.max_payload_bytes {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress payload exceeds configured byte limit".into(),
            });
        }
        Ok(())
    }

    fn validate_state(&self, state: &PersistedIngressState) -> Result<(), SecretChatError> {
        let mut seen_count = 0usize;
        if state.seen_orders.len() > MAX_STREAMS || state.pending_by_stream.len() > MAX_STREAMS {
            return Err(EncryptedIngressStore::refused(
                "durable ingress stream budget exceeded",
            ));
        }
        for (stream, orders) in &state.seen_orders {
            if stream.trim().is_empty()
                || stream.len() > MAX_ID_BYTES
                || orders.is_empty()
                || orders.iter().any(|order| *order < 0)
            {
                return Err(EncryptedIngressStore::refused(
                    "durable ingress replay history is invalid",
                ));
            }
            seen_count = seen_count
                .checked_add(orders.len())
                .filter(|total| *total <= MAX_TRACKED_ORDERS)
                .ok_or_else(|| {
                    EncryptedIngressStore::refused("durable ingress replay history budget exceeded")
                })?;
        }
        let mut pending_count = 0usize;
        let mut payload_bytes = 0usize;
        for (stream, records) in &state.pending_by_stream {
            if records.is_empty() || !state.seen_orders.contains_key(stream) {
                return Err(EncryptedIngressStore::refused(
                    "durable ingress pending stream is empty or untracked",
                ));
            }
            for (order, record) in records {
                if stream != &record.stream_id
                    || order != &record.order
                    || *order < 0
                    || !record.validated
                    || record.status == CommitReplayStatus::DuplicateIgnored
                    || record.payload.len() > self.limits.max_payload_bytes
                    || !state
                        .seen_orders
                        .get(stream)
                        .is_some_and(|orders| orders.contains(order))
                {
                    return Err(EncryptedIngressStore::refused(
                        "durable ingress pending record is inconsistent",
                    ));
                }
                pending_count = pending_count
                    .checked_add(1)
                    .filter(|total| *total <= self.limits.max_pending_records)
                    .ok_or_else(|| {
                        EncryptedIngressStore::refused(
                            "restored ingress exceeds the configured pending limit",
                        )
                    })?;
                payload_bytes = payload_bytes
                    .checked_add(record.payload.len())
                    .filter(|total| *total <= MAX_PENDING_PAYLOAD_BYTES)
                    .ok_or_else(|| {
                        EncryptedIngressStore::refused(
                            "restored ingress exceeds the aggregate payload budget",
                        )
                    })?;
            }
        }
        Ok(())
    }
}
