mod controller;
pub use controller::DurableIngressController;

use std::collections::{BTreeMap, BTreeSet};
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
    VerifiedTransport,
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
