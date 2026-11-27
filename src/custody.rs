use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::errors::SecretChatError;
use crate::storage_file;

const CUSTODY_MAGIC: &[u8; 8] = b"KSCCUS01";
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustodyMode {
    Transient,
    Bounded,
    Durable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustodyBackend {
    MemoryOnly,
    EncryptedFile,
}

/// Positive retention bounds, capped at 4096 payloads and 64 MiB of payload data.
/// The encrypted document also has a 64 MiB serialized ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyLimits {
    pub max_retained_messages: usize,
    pub max_stored_bytes: usize,
}

impl Default for CustodyLimits {
    fn default() -> Self {
        Self {
            max_retained_messages: 512,
            max_stored_bytes: 4 * 1024 * 1024,
        }
    }
}

/// A bounded payload container. Capacity refusal preserves every retained payload.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "StoredCustodyRecord")]
pub struct CustodyRecord {
    chat_id: i64,
    owner_auth_id: String,
    mode: CustodyMode,
    backend: CustodyBackend,
    limits: CustodyLimits,
    paused: bool,
    pause_reason: Option<String>,
    retained_payloads: VecDeque<Vec<u8>>,
    retained_bytes: usize,
}

impl std::fmt::Debug for CustodyRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustodyRecord")
            .field("chat_id", &self.chat_id)
            .field("mode", &self.mode)
            .field("backend", &self.backend)
            .field("limits", &self.limits)
            .field("paused", &self.paused)
            .field("retained_count", &self.retained_payloads.len())
            .field("retained_bytes", &self.retained_bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCustodyRecord {
    chat_id: i64,
    owner_auth_id: String,
    mode: CustodyMode,
    backend: CustodyBackend,
    limits: CustodyLimits,
    paused: bool,
    pause_reason: Option<String>,
    retained_payloads: VecDeque<Vec<u8>>,
    retained_bytes: usize,
}

impl TryFrom<StoredCustodyRecord> for CustodyRecord {
    type Error = SecretChatError;

    fn try_from(stored: StoredCustodyRecord) -> Result<Self, Self::Error> {
        let record = Self {
            chat_id: stored.chat_id,
            owner_auth_id: stored.owner_auth_id,
            mode: stored.mode,
            backend: stored.backend,
            limits: stored.limits,
            paused: stored.paused,
            pause_reason: stored.pause_reason,
            retained_payloads: stored.retained_payloads,
            retained_bytes: stored.retained_bytes,
        };
        record.validate()?;
        Ok(record)
    }
}

impl CustodyRecord {
    pub fn new(
        chat_id: i64,
        owner_auth_id: impl Into<String>,
        mode: CustodyMode,
        backend: CustodyBackend,
        limits: CustodyLimits,
    ) -> Self {
        Self {
            chat_id,
            owner_auth_id: owner_auth_id.into(),
            mode,
            backend,
            limits,
            paused: false,
            pause_reason: None,
            retained_payloads: VecDeque::new(),
            retained_bytes: 0,
        }
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    pub fn owner_auth_id(&self) -> &str {
        &self.owner_auth_id
    }

    pub fn mode(&self) -> CustodyMode {
        self.mode
    }

    pub fn backend(&self) -> CustodyBackend {
        self.backend
    }

    pub fn limits(&self) -> CustodyLimits {
        self.limits
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self, reason: impl Into<String>) {
        self.paused = true;
        self.pause_reason = Some(reason.into());
    }

    /// Clear the local pause flag. The caller must establish protocol continuity;
    /// this payload container cannot validate a transport or protocol checkpoint.
    pub fn resume(&mut self) {
        self.paused = false;
        self.pause_reason = None;
    }

    pub fn retained_count(&self) -> usize {
        self.retained_payloads.len()
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn push_payload(&mut self, payload: &[u8]) -> Result<(), SecretChatError> {
        self.validate()?;
        if self.paused {
            return Err(SecretChatError::ChatNotReady {
                chat_id: self.chat_id,
                state: "paused".into(),
            });
        }
        if payload.len() > self.limits.max_stored_bytes {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id),
                reason: "payload exceeds configured custody byte ceiling".into(),
            });
        }
        if self.retained_payloads.len() >= self.limits.max_retained_messages
            || payload.len() > self.limits.max_stored_bytes - self.retained_bytes
        {
            self.pause("custody capacity reached; retained obligations require settlement");
            return Err(SecretChatError::ChatNotReady {
                chat_id: self.chat_id,
                state: "custody capacity reached".into(),
            });
        }

        self.retained_bytes += payload.len();
        self.retained_payloads.push_back(payload.to_vec());
        Ok(())
    }

    fn validate(&self) -> Result<(), SecretChatError> {
        let valid_bytes = self
            .retained_payloads
            .iter()
            .try_fold(0usize, |total, payload| total.checked_add(payload.len()));
        if self.owner_auth_id.trim().is_empty()
            || self.owner_auth_id.len() > 4096
            || self.limits.max_retained_messages == 0
            || self.limits.max_retained_messages > 4096
            || self.limits.max_stored_bytes == 0
            || self.limits.max_stored_bytes > storage_file::MAX_STORE_BYTES
            || self.retained_payloads.len() > self.limits.max_retained_messages
            || self.retained_bytes > self.limits.max_stored_bytes
            || valid_bytes != Some(self.retained_bytes)
            || self.paused != self.pause_reason.is_some()
        {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(self.chat_id),
                reason: "custody identity, limits, pause state or payload accounting is invalid"
                    .into(),
            });
        }
        Ok(())
    }

    fn validate_persistence(&self) -> Result<(), SecretChatError> {
        self.validate()?;
        if self.backend != CustodyBackend::EncryptedFile || self.mode == CustodyMode::Transient {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id),
                reason: "volatile custody cannot be persisted in an encrypted file".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
struct CustodyKey([u8; 32]);

/// Explicit-path AES-256-GCM custody store (never falls back to process CWD).
/// Successful record access or save binds a handle to that authorization.
/// Cleanup preserves the binding; open a new handle to adopt another owner.
pub struct CustodyStore {
    path: PathBuf,
    key: CustodyKey,
    owner_auth_id: OnceLock<String>,
    uncertain: bool,
}

impl std::fmt::Debug for CustodyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustodyStore")
            .field("has_path", &!self.path.as_os_str().is_empty())
            .finish()
    }
}

impl CustodyStore {
    pub fn open_durable(path: impl AsRef<Path>, key: [u8; 32]) -> Result<Self, SecretChatError> {
        let path = storage_file::absolute_store_path(path.as_ref())
            .map_err(|_| SecretChatError::StorageRequired)?;
        Ok(Self {
            path,
            key: CustodyKey(key),
            owner_auth_id: OnceLock::new(),
            uncertain: false,
        })
    }

    pub fn save_record(&mut self, record: &CustodyRecord) -> Result<(), SecretChatError> {
        self.ensure_certain()?;
        record.validate_persistence()?;
        self.ensure_owner(&record.owner_auth_id)?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| self.storage_error("failed to acquire custody store lock"))?;
        let mut records = self.load_all()?;
        if records
            .iter()
            .any(|existing| existing.owner_auth_id != record.owner_auth_id)
        {
            return Err(self.storage_error("custody store belongs to a different authorization"));
        }
        records.retain(|r| r.chat_id != record.chat_id);
        records.push(record.clone());

        let plaintext = Zeroizing::new(serde_json::to_vec(&records).map_err(|_| {
            SecretChatError::StoreCorrupt {
                chat_id: Some(record.chat_id),
                reason: "failed to serialize custody record".into(),
            }
        })?);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes).map_err(|err| SecretChatError::EntropyUnavailable {
            reason: err.to_string(),
        })?;
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.key.0));
        let ciphertext = cipher
            .encrypt(&Nonce::from(nonce_bytes), plaintext.as_slice())
            .map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: Some(record.chat_id),
                reason: "failed to encrypt custody record".into(),
            })?;

        let mut envelope = Vec::with_capacity(CUSTODY_MAGIC.len() + NONCE_LEN + ciphertext.len());
        envelope.extend_from_slice(CUSTODY_MAGIC);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);

        if let Err(failure) = storage_file::atomic_replace(&self.path, &envelope) {
            self.uncertain = failure.replaced;
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(record.chat_id),
                reason: format!(
                    "failed to commit custody store ({:?}); reopen after an uncertain replacement",
                    failure.error.kind()
                ),
            });
        }
        self.bind_owner(&record.owner_auth_id)?;
        Ok(())
    }

    pub fn load_record(
        &self,
        chat_id: i64,
        expected_owner_auth_id: &str,
    ) -> Result<CustodyRecord, SecretChatError> {
        self.ensure_certain()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| self.storage_error("failed to acquire custody store lock"))?;
        let records = self.load_all()?;
        let record = records
            .into_iter()
            .find(|r| r.chat_id == chat_id)
            .ok_or(SecretChatError::UnknownChat { chat_id })?;
        if record.owner_auth_id != expected_owner_auth_id {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(chat_id),
                reason: "custody record belongs to a different authorization".into(),
            });
        }
        self.bind_owner(expected_owner_auth_id)?;
        Ok(record)
    }

    pub fn cleanup_owned_files(&mut self) -> Result<(), SecretChatError> {
        self.ensure_certain()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| self.storage_error("failed to acquire custody store lock"))?;
        // A filename alone is not evidence of ownership. Authenticate before deletion.
        let records = self.load_all()?;
        if let Some(record) = records.first() {
            self.bind_owner(&record.owner_auth_id)?;
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                if storage_file::sync_parent(&self.path).is_err() {
                    self.uncertain = true;
                    return Err(self.storage_error(
                        "custody deletion durability is uncertain; reopen the store",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(self.storage_error("failed to remove owned custody store file")),
        }
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<CustodyRecord>, SecretChatError> {
        let Some(raw) = storage_file::read_store(&self.path)
            .map_err(|_| self.storage_error("failed to read custody store"))?
        else {
            return Ok(Vec::new());
        };
        if raw.len() < CUSTODY_MAGIC.len() + NONCE_LEN + 16 || &raw[..8] != CUSTODY_MAGIC {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "custody envelope header is invalid".into(),
            });
        }
        let nonce_arr: [u8; NONCE_LEN] =
            raw[8..8 + NONCE_LEN]
                .try_into()
                .map_err(|_| SecretChatError::StoreCorrupt {
                    chat_id: None,
                    reason: "custody nonce length invalid".into(),
                })?;
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.key.0));
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(&Nonce::from(nonce_arr), &raw[8 + NONCE_LEN..])
                .map_err(|_| SecretChatError::StoreCorrupt {
                    chat_id: None,
                    reason: "custody authentication tag check failed".into(),
                })?,
        );
        let records: Vec<CustodyRecord> =
            serde_json::from_slice(&plaintext).map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "custody store document failed schema decoding".into(),
            })?;
        let mut chat_ids = BTreeSet::new();
        let owner = records.first().map(|record| record.owner_auth_id.as_str());
        for record in &records {
            record.validate_persistence()?;
            if !chat_ids.insert(record.chat_id) || owner != Some(record.owner_auth_id.as_str()) {
                return Err(self.storage_error("custody store identities are inconsistent"));
            }
        }
        if let Some(owner) = owner {
            self.ensure_owner(owner)?;
        }
        Ok(records)
    }

    fn ensure_owner(&self, owner: &str) -> Result<(), SecretChatError> {
        if self.owner_auth_id.get().is_some_and(|bound| bound != owner) {
            return Err(self.storage_error(
                "custody handle is bound to a different authorization; open a new handle",
            ));
        }
        Ok(())
    }

    fn bind_owner(&self, owner: &str) -> Result<(), SecretChatError> {
        self.owner_auth_id.get_or_init(|| owner.to_owned());
        self.ensure_owner(owner)
    }

    fn storage_error(&self, reason: &str) -> SecretChatError {
        SecretChatError::StoreCorrupt {
            chat_id: None,
            reason: reason.into(),
        }
    }

    fn ensure_certain(&self) -> Result<(), SecretChatError> {
        if self.uncertain {
            return Err(self.storage_error("custody commit outcome is uncertain; reopen the store"));
        }
        Ok(())
    }
}
