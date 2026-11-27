use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::errors::SecretChatError;
use crate::storage_file;

const ENVELOPE_MAGIC: &[u8; 8] = b"KSCING01";
const NONCE_LEN: usize = 12;
const MAX_TRACKED_ORDERS: usize = 65_536;
const MAX_STREAMS: usize = 4096;
const MAX_ID_BYTES: usize = 1024;
const MAX_PENDING_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngressOrigin {
    LiveSocket,
    DifferenceRecovery,
    ChannelDifferenceRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitReplayStatus {
    Committed,
    DuplicateIgnored,
    Replayed,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressUpdate {
    pub stream_id: String,
    pub order: i64,
    pub provenance: IngressOrigin,
    pub associated_chat_id: Option<i64>,
    pub payload: Vec<u8>,
}

impl std::fmt::Debug for IngressUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngressUpdate")
            .field("order", &self.order)
            .field("provenance", &self.provenance)
            .field("associated_chat_id", &self.associated_chat_id)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

/// Durable ingress record matching `specs/019-rust-anything-full-rewrite/data-model.md`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableIngressRecord {
    pub stream_id: String,
    pub order: i64,
    pub provenance: IngressOrigin,
    pub validated: bool,
    pub status: CommitReplayStatus,
    pub associated_chat_id: Option<i64>,
    pub payload: Vec<u8>,
}

impl std::fmt::Debug for DurableIngressRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableIngressRecord")
            .field("order", &self.order)
            .field("provenance", &self.provenance)
            .field("validated", &self.validated)
            .field("status", &self.status)
            .field("associated_chat_id", &self.associated_chat_id)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

/// Positive limits, capped at 4096 pending records and 1 MiB per payload.
/// Additional hard ceilings are 16 MiB of aggregate pending payload, 4096 streams,
/// 65,536 retained deduplication identities, and a 64 MiB encrypted document.
/// Reaching a ceiling refuses new custody; replay identities are never evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressLimits {
    pub max_pending_records: usize,
    pub max_payload_bytes: usize,
}

impl Default for IngressLimits {
    fn default() -> Self {
        Self {
            max_pending_records: 1024,
            max_payload_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
struct StoreKey([u8; 32]);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedIngressState {
    owner_auth_id: String,
    #[serde(deserialize_with = "deserialize_seen_orders")]
    seen_orders: BTreeMap<String, BTreeSet<i64>>,
    #[serde(deserialize_with = "deserialize_pending_streams")]
    pending_by_stream: BTreeMap<String, BTreeMap<i64, DurableIngressRecord>>,
}

#[derive(Deserialize)]
#[serde(try_from = "Vec<i64>")]
struct StoredOrders(BTreeSet<i64>);

impl TryFrom<Vec<i64>> for StoredOrders {
    type Error = &'static str;

    fn try_from(orders: Vec<i64>) -> Result<Self, Self::Error> {
        let mut unique = BTreeSet::new();
        for order in orders {
            if !unique.insert(order) {
                return Err("duplicate durable ingress replay identity");
            }
        }
        Ok(Self(unique))
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct StoredPendingRecords(
    #[serde(deserialize_with = "deserialize_unique_map")] BTreeMap<i64, DurableIngressRecord>,
);

fn deserialize_seen_orders<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeSet<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let streams: BTreeMap<String, StoredOrders> = deserialize_unique_map(deserializer)?;
    Ok(streams
        .into_iter()
        .map(|(stream, orders)| (stream, orders.0))
        .collect())
}

fn deserialize_pending_streams<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<i64, DurableIngressRecord>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let streams: BTreeMap<String, StoredPendingRecords> = deserialize_unique_map(deserializer)?;
    Ok(streams
        .into_iter()
        .map(|(stream, records)| (stream, records.0))
        .collect())
}

fn deserialize_unique_map<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(std::marker::PhantomData<(K, V)>);

    impl<'de, K, V> serde::de::Visitor<'de> for UniqueMap<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map with unique durable ingress identities")
        }

        fn visit_map<A>(self, mut input: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut values = BTreeMap::new();
            while let Some(key) = input.next_key()? {
                match values.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(input.next_value()?);
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {
                        return Err(serde::de::Error::custom(
                            "duplicate durable ingress map identity",
                        ));
                    }
                }
            }
            Ok(values)
        }
    }

    deserializer.deserialize_map(UniqueMap(std::marker::PhantomData))
}

/// AES-256-GCM authenticated, atomically replaced on-disk store for durable ingress state.
pub struct EncryptedIngressStore {
    path: PathBuf,
    key: StoreKey,
    revision: Option<[u8; 32]>,
    uncertain: bool,
}

impl EncryptedIngressStore {
    pub fn open(path: impl AsRef<Path>, key: [u8; 32]) -> Result<Self, SecretChatError> {
        let path = storage_file::absolute_store_path(path.as_ref()).map_err(|_| {
            SecretChatError::IngressRefused {
                reason:
                    "ingress store requires an absolute file path and an existing parent directory"
                        .into(),
            }
        })?;
        Ok(Self {
            path,
            key: StoreKey(key),
            revision: None,
            uncertain: false,
        })
    }

    fn save(&mut self, state: &PersistedIngressState) -> Result<(), SecretChatError> {
        self.ensure_certain()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| Self::refused("failed to acquire durable ingress store lock"))?;
        self.check_revision()?;
        let plaintext = Zeroizing::new(serde_json::to_vec(state).map_err(|_| {
            SecretChatError::IngressRefused {
                reason: "failed to encode durable ingress state".into(),
            }
        })?);
        if plaintext.len() > storage_file::MAX_STORE_BYTES - ENVELOPE_MAGIC.len() - NONCE_LEN - 16 {
            return Err(Self::refused(
                "durable ingress serialized state exceeds the store byte ceiling",
            ));
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes).map_err(|_| SecretChatError::IngressRefused {
            reason: "OS entropy unavailable for ingress encryption".into(),
        })?;

        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.key.0));
        let ciphertext = cipher
            .encrypt(&Nonce::from(nonce_bytes), plaintext.as_slice())
            .map_err(|_| SecretChatError::IngressRefused {
                reason: "failed to authenticate durable ingress state".into(),
            })?;

        let mut envelope = Vec::with_capacity(ENVELOPE_MAGIC.len() + NONCE_LEN + ciphertext.len());
        envelope.extend_from_slice(ENVELOPE_MAGIC);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);

        if let Err(failure) = storage_file::atomic_replace(&self.path, &envelope) {
            self.uncertain = failure.replaced;
            return Err(Self::refused(&format!(
                "failed to commit durable ingress ({:?}); reopen after an uncertain replacement",
                failure.error.kind()
            )));
        }
        self.revision = Some(Sha256::digest(&envelope).into());
        Ok(())
    }

    fn load(&mut self) -> Result<Option<PersistedIngressState>, SecretChatError> {
        self.ensure_certain()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| Self::refused("failed to acquire durable ingress store lock"))?;
        let Some(raw) = storage_file::read_store(&self.path)
            .map_err(|_| Self::refused("failed to read durable ingress store"))?
        else {
            self.revision = None;
            return Ok(None);
        };
        if raw.len() < ENVELOPE_MAGIC.len() + NONCE_LEN + 16 || &raw[..8] != ENVELOPE_MAGIC {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "durable ingress envelope header is invalid".into(),
            });
        }
        let nonce_arr: [u8; NONCE_LEN] =
            raw[8..8 + NONCE_LEN]
                .try_into()
                .map_err(|_| SecretChatError::StoreCorrupt {
                    chat_id: None,
                    reason: "durable ingress nonce length invalid".into(),
                })?;
        let nonce = Nonce::from(nonce_arr);
        let ciphertext = &raw[8 + NONCE_LEN..];
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.key.0));
        let plaintext = Zeroizing::new(cipher.decrypt(&nonce, ciphertext).map_err(|_| {
            SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "durable ingress authentication tag check failed".into(),
            }
        })?);
        let state: PersistedIngressState =
            serde_json::from_slice(&plaintext).map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "durable ingress document failed schema decoding".into(),
            })?;
        self.revision = Some(Sha256::digest(&raw).into());
        Ok(Some(state))
    }

    fn check_revision(&self) -> Result<(), SecretChatError> {
        let current = storage_file::read_store(&self.path)
            .map_err(|_| Self::refused("failed to read durable ingress store"))?
            .map(|raw| <[u8; 32]>::from(Sha256::digest(&raw)));
        if current != self.revision {
            return Err(Self::refused(
                "durable ingress store changed; restore it before further operations",
            ));
        }
        Ok(())
    }

    fn ensure_current(&self) -> Result<(), SecretChatError> {
        self.ensure_certain()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| Self::refused("failed to acquire durable ingress store lock"))?;
        self.check_revision()
    }

    fn ensure_certain(&self) -> Result<(), SecretChatError> {
        if self.uncertain {
            return Err(Self::refused(
                "durable ingress commit outcome is uncertain; reopen the store",
            ));
        }
        Ok(())
    }

    fn refused(reason: &str) -> SecretChatError {
        SecretChatError::IngressRefused {
            reason: reason.into(),
        }
    }
}

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
