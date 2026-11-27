use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use zeroize::Zeroizing;

use crate::errors::SecretChatError;
use crate::storage_file;

use super::*;

pub struct FileStorage {
    path: PathBuf,
    account_binding: String,
    key: Zeroizing<[u8; 32]>,
    protected: bool,
    uncertain: Mutex<bool>,
}

impl FileStorage {
    pub fn open(
        path: impl AsRef<Path>,
        key: [u8; 32],
        account_binding: impl Into<String>,
    ) -> Result<Self> {
        Self::open_with(
            path.as_ref(),
            Zeroizing::new(key),
            account_binding.into(),
            false,
        )
    }

    fn open_with(
        path: &Path,
        key: Zeroizing<[u8; 32]>,
        account_binding: String,
        protected: bool,
    ) -> Result<Self> {
        validate_binding(&account_binding)?;
        let path = storage_file::absolute_store_path(path)
            .map_err(|_| SecretChatError::StorageRequired)?;
        let store = Self {
            path,
            account_binding,
            key,
            protected,
            uncertain: Mutex::new(false),
        };
        store.access(|_| Ok(((), false)), true)?;
        Ok(store)
    }

    fn magic(&self) -> &[u8; 8] {
        if self.protected {
            PROTECTED_MAGIC
        } else {
            MAGIC
        }
    }

    fn aad(&self) -> Vec<u8> {
        [self.magic().as_slice(), self.account_binding.as_bytes()].concat()
    }

    fn access<R>(
        &self,
        action: impl FnOnce(&mut PersistedStoreSnapshot) -> Result<(R, bool)>,
        initialize: bool,
    ) -> Result<R> {
        let mut uncertain = self
            .uncertain
            .lock()
            .map_err(|_| corrupt("file store lock was poisoned"))?;
        if *uncertain {
            return Err(corrupt(
                "secret-chat commit outcome is uncertain; reopen the store",
            ));
        }
        self.check_permissions()?;
        let _lock = storage_file::lock_store(&self.path)
            .map_err(|_| corrupt("secret-chat store lock is unavailable or invalid"))?;
        self.check_permissions()?;
        let raw = storage_file::read_store(&self.path)
            .map_err(|_| corrupt("failed to read secret-chat store"))?;
        let was_missing = raw.is_none();
        let mut state = match raw {
            Some(raw) => self.decode(&raw)?,
            None if initialize => PersistedStoreSnapshot::empty(self.account_binding.clone()),
            None => return Err(corrupt("secret-chat store disappeared; reopen the store")),
        };
        let (result, changed) = action(&mut state)?;
        if was_missing || changed {
            let envelope = self.encode(&state)?;
            if let Err(failure) = storage_file::atomic_replace(&self.path, &envelope) {
                *uncertain = failure.replaced;
                return Err(corrupt(
                    "failed to commit secret-chat store; reopen after an uncertain replacement",
                ));
            }
        }
        Ok(result)
    }

    fn decode(&self, raw: &[u8]) -> Result<PersistedStoreSnapshot> {
        if !raw.starts_with(self.magic()) || raw.len() < 8 + NONCE_LEN + TAG_LEN {
            return Err(corrupt("invalid secret-chat store envelope"));
        }
        let nonce = <[u8; NONCE_LEN]>::try_from(&raw[8..8 + NONCE_LEN])
            .map_err(|_| corrupt("invalid secret-chat store nonce"))?;
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*self.key));
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &Nonce::from(nonce),
                    Payload {
                        msg: &raw[8 + NONCE_LEN..],
                        aad: &self.aad(),
                    },
                )
                .map_err(|_| corrupt("secret-chat store authentication failed"))?,
        );
        let state: PersistedStoreSnapshot = serde_json::from_slice(&plaintext)
            .map_err(|_| corrupt("invalid secret-chat store document"))?;
        state.validate(&self.account_binding)?;
        Ok(state)
    }

    fn encode(&self, state: &PersistedStoreSnapshot) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(state).map_err(|_| corrupt("failed to encode secret-chat store"))?,
        );
        if plaintext.len() > storage_file::MAX_STORE_BYTES - 8 - NONCE_LEN - TAG_LEN {
            return Err(corrupt("secret-chat store exceeds its byte limit"));
        }
        let mut nonce = [0; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "OS randomness unavailable".into(),
        })?;
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*self.key));
        let encrypted = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &self.aad(),
                },
            )
            .map_err(|_| corrupt("failed to encrypt secret-chat store"))?;
        Ok([self.magic().as_slice(), &nonce, &encrypted].concat())
    }

    // The caller must keep the parent directory private, including its ACL on Windows.
    fn check_permissions(&self) -> Result<()> {
        if !self.protected {
            return Ok(());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut lock_name = self
                .path
                .file_name()
                .ok_or(SecretChatError::StorageRequired)?
                .to_os_string();
            lock_name.push(".lock");
            for path in [&self.path, &self.path.with_file_name(lock_name)] {
                match std::fs::symlink_metadata(path) {
                    Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o077 == 0 => (),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
                    _ => {
                        return Err(corrupt(
                            "protected secret-chat store requires private regular files",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl SecretChatStore for FileStorage {
    fn load_all(&self) -> Result<PersistedStoreSnapshot> {
        self.access(|state| Ok((state.clone(), false)), false)
    }
    fn get(&self, id: i32) -> Result<Option<PersistedChatRecord>> {
        self.access(|state| Ok((state.chats.remove(&id), false)), false)
    }
    fn save(&self, record: &PersistedChatRecord) -> Result<()> {
        self.access(
            |state| {
                Ok(((), {
                    save_record(state, record, None)?;
                    true
                }))
            },
            false,
        )
    }
    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<()> {
        record.checkpoint = self.access(
            |state| Ok((save_record(state, record, Some(expected))?, true)),
            false,
        )?;
        Ok(())
    }
    fn delete(&self, id: i32) -> Result<()> {
        self.access(|state| Ok(((), delete_record(state, id))), false)
    }
    fn mark_transient_suspended(&self, id: i32) -> Result<()> {
        self.access(|state| Ok(((), suspend_record(state, id)?)), false)
    }
}

/// An account-bound file with its own authenticated format and private-file checks on Unix.
/// On Windows the caller must provide a private directory ACL.
pub struct ProtectedFileStorage(FileStorage);

impl ProtectedFileStorage {
    pub fn open(
        path: impl AsRef<Path>,
        key: [u8; 32],
        account_binding: impl Into<String>,
    ) -> Result<Self> {
        FileStorage::open_with(
            path.as_ref(),
            Zeroizing::new(key),
            account_binding.into(),
            true,
        )
        .map(Self)
    }
}

impl SecretChatStore for ProtectedFileStorage {
    fn load_all(&self) -> Result<PersistedStoreSnapshot> {
        self.0.load_all()
    }
    fn get(&self, id: i32) -> Result<Option<PersistedChatRecord>> {
        self.0.get(id)
    }
    fn save(&self, record: &PersistedChatRecord) -> Result<()> {
        self.0.save(record)
    }
    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<()> {
        self.0.save_checkpoint(expected, record)
    }
    fn delete(&self, id: i32) -> Result<()> {
        self.0.delete(id)
    }
    fn mark_transient_suspended(&self, id: i32) -> Result<()> {
        self.0.mark_transient_suspended(id)
    }
}
