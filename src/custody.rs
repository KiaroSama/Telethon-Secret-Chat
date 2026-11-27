use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::errors::SecretChatError;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Custody record aligned with `specs/019-rust-anything-full-rewrite/data-model.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        if self.limits.max_retained_messages == 0 {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id),
                reason: "custody message retention bound is zero".into(),
            });
        }

        while self.retained_payloads.len() >= self.limits.max_retained_messages
            || self.retained_bytes + payload.len() > self.limits.max_stored_bytes
        {
            if let Some(evicted) = self.retained_payloads.pop_front() {
                self.retained_bytes = self.retained_bytes.saturating_sub(evicted.len());
            } else {
                break;
            }
        }

        self.retained_bytes += payload.len();
        self.retained_payloads.push_back(payload.to_vec());
        Ok(())
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
struct CustodyKey([u8; 32]);

/// Explicit-path AES-256-GCM custody store (never falls back to process CWD).
pub struct CustodyStore {
    path: PathBuf,
    key: CustodyKey,
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
        let p = path.as_ref();
        // Reject bare relative paths to forbid silent CWD fallback (FR-006 / T014).
        if !p.is_absolute() {
            return Err(SecretChatError::StorageRequired);
        }
        let parent = p
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .ok_or(SecretChatError::StorageRequired)?;
        if !parent.exists() {
            return Err(SecretChatError::StorageRequired);
        }
        Ok(Self {
            path: p.to_path_buf(),
            key: CustodyKey(key),
        })
    }

    pub fn save_record(&mut self, record: &CustodyRecord) -> Result<(), SecretChatError> {
        let mut records = if self.path.exists() {
            self.load_all()?
        } else {
            Vec::new()
        };
        records.retain(|r| r.chat_id != record.chat_id);
        records.push(record.clone());

        let plaintext =
            serde_json::to_vec(&records).map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: Some(record.chat_id),
                reason: "failed to serialize custody record".into(),
            })?;

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

        let parent = self.path.parent().ok_or(SecretChatError::StorageRequired)?;
        let mut tmp_name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(".tmp");
        let tmp_path = parent.join(tmp_name);

        std::fs::write(&tmp_path, &envelope).map_err(|_| SecretChatError::StoreCorrupt {
            chat_id: Some(record.chat_id),
            reason: "failed to stage custody store update".into(),
        })?;
        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        std::fs::rename(&tmp_path, &self.path).map_err(|_| {
            let _ = std::fs::remove_file(&tmp_path);
            SecretChatError::StoreCorrupt {
                chat_id: Some(record.chat_id),
                reason: "failed to replace custody store atomically".into(),
            }
        })?;
        Ok(())
    }

    pub fn load_record(
        &self,
        chat_id: i64,
        expected_owner_auth_id: &str,
    ) -> Result<CustodyRecord, SecretChatError> {
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
        Ok(record)
    }

    pub fn cleanup_owned_files(&mut self) -> Result<(), SecretChatError> {
        if self.path.exists() {
            std::fs::remove_file(&self.path).map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "failed to remove owned custody store file".into(),
            })?;
        }
        let parent = self.path.parent().ok_or(SecretChatError::StorageRequired)?;
        let mut tmp_name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(".tmp");
        let tmp_path = parent.join(tmp_name);
        if tmp_path.exists() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<CustodyRecord>, SecretChatError> {
        let raw = std::fs::read(&self.path).map_err(|_| SecretChatError::StoreCorrupt {
            chat_id: None,
            reason: "failed to read custody store".into(),
        })?;
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
        let plaintext = cipher
            .decrypt(&Nonce::from(nonce_arr), &raw[8 + NONCE_LEN..])
            .map_err(|_| SecretChatError::StoreCorrupt {
                chat_id: None,
                reason: "custody authentication tag check failed".into(),
            })?;
        serde_json::from_slice(&plaintext).map_err(|_| SecretChatError::StoreCorrupt {
            chat_id: None,
            reason: "custody store document failed schema decoding".into(),
        })
    }
}
