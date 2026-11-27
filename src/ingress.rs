use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::errors::SecretChatError;

const ENVELOPE_MAGIC: &[u8; 8] = b"KSCING01";
const NONCE_LEN: usize = 12;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressUpdate {
    pub stream_id: String,
    pub order: i64,
    pub provenance: IngressOrigin,
    pub associated_chat_id: Option<i64>,
    pub payload: Vec<u8>,
}

/// Durable ingress record matching `specs/019-rust-anything-full-rewrite/data-model.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableIngressRecord {
    pub stream_id: String,
    pub order: i64,
    pub provenance: IngressOrigin,
    pub validated: bool,
    pub status: CommitReplayStatus,
    pub associated_chat_id: Option<i64>,
    pub payload: Vec<u8>,
}

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
struct PersistedIngressState {
    owner_auth_id: String,
    seen_orders: BTreeMap<String, BTreeSet<i64>>,
    pending_by_stream: BTreeMap<String, BTreeMap<i64, DurableIngressRecord>>,
}

/// AES-256-GCM authenticated, atomically replaced on-disk store for durable ingress state.
pub struct EncryptedIngressStore {
    path: PathBuf,
    key: StoreKey,
}

impl EncryptedIngressStore {
    pub fn open(path: impl AsRef<Path>, key: [u8; 32]) -> Result<Self, SecretChatError> {
        let p = path.as_ref();
        if p.as_os_str().is_empty() {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress store path cannot be empty".into(),
            });
        }
        Ok(Self {
            path: p.to_path_buf(),
            key: StoreKey(key),
        })
    }

    fn save(&self, state: &PersistedIngressState) -> Result<(), SecretChatError> {
        if self.path.is_dir() {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress store path points to a directory".into(),
            });
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| SecretChatError::IngressRefused {
                reason: "ingress store path must have an explicit parent directory".into(),
            })?;
        if !parent.exists() {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress store parent directory is unavailable".into(),
            });
        }

        let plaintext = serde_json::to_vec(state).map_err(|_| SecretChatError::IngressRefused {
            reason: "failed to encode durable ingress state".into(),
        })?;

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

        let mut tmp_name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(".tmp");
        let tmp_path = parent.join(tmp_name);

        std::fs::write(&tmp_path, &envelope).map_err(|_| SecretChatError::IngressRefused {
            reason: "failed to write durable ingress staging file".into(),
        })?;

        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        std::fs::rename(&tmp_path, &self.path).map_err(|_| {
            let _ = std::fs::remove_file(&tmp_path);
            SecretChatError::IngressRefused {
                reason: "failed to atomically replace durable ingress store".into(),
            }
        })?;
        Ok(())
    }

    fn load(&self) -> Result<Option<PersistedIngressState>, SecretChatError> {
        if self.path.is_dir() {
            return Err(SecretChatError::IngressRefused {
                reason: "ingress store path points to a directory".into(),
            });
        }
        if !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read(&self.path).map_err(|_| SecretChatError::IngressRefused {
            reason: "failed to read durable ingress store".into(),
        })?;
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
        let plaintext =
            cipher
                .decrypt(&nonce, ciphertext)
                .map_err(|_| SecretChatError::StoreCorrupt {
                    chat_id: None,
                    reason: "durable ingress authentication tag check failed".into(),
                })?;
        let state: PersistedIngressState =
            serde_json::from_slice(&plaintext).map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "durable ingress document failed schema decoding".into(),
            })?;
        Ok(Some(state))
    }
}

/// Coordinates validated, owner-bound, durable capture and ordered replay on top of
/// `grammers-mtsender`'s durable ingress boundary.
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
            .field("owner_auth_id", &self.owner_auth_id)
            .field("streams", &self.seen_orders.len())
            .finish()
    }
}

impl DurableIngressController {
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
        store: EncryptedIngressStore,
        limits: IngressLimits,
    ) -> Result<Self, SecretChatError> {
        let expected_owner = owner_auth_id.into();
        let persisted = store.load()?;
        let mut controller = Self::new(expected_owner.clone(), store, limits);
        if let Some(state) = persisted {
            if state.owner_auth_id != expected_owner {
                return Err(SecretChatError::IngressRefused {
                    reason: "durable ingress store belongs to a different authorization".into(),
                });
            }
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

        let seen = self
            .seen_orders
            .entry(update.stream_id.clone())
            .or_default();
        if seen.contains(&update.order) {
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
            seen_orders: next_seen.clone(),
            pending_by_stream: next_pending.clone(),
        };
        self.store.save(&snapshot)?;

        self.seen_orders = next_seen;
        self.pending_by_stream = next_pending;
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
        barrier().await?;
        self.capture_and_commit(update)
    }

    pub fn drain_in_order(&mut self, stream_id: &str) -> Vec<DurableIngressRecord> {
        let drained: Vec<DurableIngressRecord> = self
            .pending_by_stream
            .remove(stream_id)
            .map(|map| map.into_values().collect())
            .unwrap_or_default();
        if !drained.is_empty() {
            let snapshot = PersistedIngressState {
                owner_auth_id: self.owner_auth_id.clone(),
                seen_orders: self.seen_orders.clone(),
                pending_by_stream: self.pending_by_stream.clone(),
            };
            let _ = self.store.save(&snapshot);
        }
        drained
    }

    fn validate_update(&self, update: &IngressUpdate) -> Result<(), SecretChatError> {
        if update.stream_id.trim().is_empty() {
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
        let total_pending: usize = self.pending_by_stream.values().map(BTreeMap::len).sum();
        if total_pending >= self.limits.max_pending_records {
            return Err(SecretChatError::IngressRefused {
                reason: "durable ingress pending queue limit reached".into(),
            });
        }
        Ok(())
    }
}
