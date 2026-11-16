//! Caller-key authenticated whole-state custody. Only ciphertext and a binding
//! digest reach the underlying atomic, owner-locked file backend.
use crate::{
    binding::AuthorizationBinding,
    crypto::random_bytes,
    error::{Error, Result},
    storage::{FileStorage, SecretRecord, State, Storage, StoreMode},
};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use std::path::Path;
use zeroize::Zeroizing;
const BLOB: &str = "aead/ingress-v1";
const MAGIC: &[u8] = b"KIARO-INGRESS-RUST-1\0";
pub struct EncryptedIngressStorage {
    backend: FileStorage,
    key: Zeroizing<[u8; 32]>,
    binding: AuthorizationBinding,
    max_bytes: usize,
    state: State,
    failed: bool,
}
impl EncryptedIngressStorage {
    #[cfg(test)]
    pub(crate) fn fail_next_commit(&mut self, fault: crate::storage::CommitFault) {
        self.backend.fail_next_commit(fault);
    }
    pub fn open(
        path: impl AsRef<Path>,
        key: [u8; 32],
        binding: AuthorizationBinding,
        max_bytes: usize,
    ) -> Result<Self> {
        binding.validate()?;
        if !(4096..=64 * 1024 * 1024).contains(&max_bytes) {
            return Err(Error::Capacity);
        }
        let absolute = std::path::absolute(path).map_err(|_| Error::Storage)?;
        if absolute
            .ancestors()
            .any(|path| std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()))
        {
            return Err(Error::Storage);
        }
        let backend = FileStorage::open(absolute)?;
        let mut state = State::empty(binding.digest())?;
        state.mode = StoreMode::Ingress;
        let mut store = Self {
            backend,
            key: Zeroizing::new(key),
            binding,
            max_bytes,
            state,
            failed: false,
        };
        if let Some(outer) = store.backend.load()? {
            store.state = store.unseal(&outer)?;
        } else {
            let state = store.state.clone();
            store.commit(&state)?;
        }
        Ok(store)
    }
    pub fn binding(&self) -> &AuthorizationBinding {
        &self.binding
    }
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
    fn aad(&self) -> Vec<u8> {
        let mut aad = MAGIC.to_vec();
        aad.extend_from_slice(self.binding.digest().as_bytes());
        aad
    }
    fn unseal(&self, outer: &State) -> Result<State> {
        if outer.authorization != self.binding.digest()
            || outer.mode != StoreMode::Ingress
            || !outer.chats.is_empty()
            || !outer.settings.is_empty()
            || outer.private_records.len() != 1
        {
            return Err(Error::InvalidState);
        }
        let blob = outer
            .private_records
            .get(BLOB)
            .ok_or(Error::InvalidState)?
            .as_bytes();
        if blob.len() < 28 || blob.len() > self.max_bytes + 28 {
            return Err(Error::InvalidState);
        }
        let nonce: [u8; 12] = blob[..12].try_into().map_err(|_| Error::InvalidState)?;
        let cipher = Aes256Gcm::new_from_slice(&*self.key).map_err(|_| Error::InvalidState)?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &Nonce::from(nonce),
                    Payload {
                        msg: &blob[12..],
                        aad: &self.aad(),
                    },
                )
                .map_err(|_| Error::Authorization)?,
        );
        if plaintext.len() + 256 > self.max_bytes {
            return Err(Error::Capacity);
        }
        let state: State = serde_json::from_slice(&plaintext).map_err(|_| Error::InvalidState)?;
        if state.authorization != self.binding.digest() || state.mode != StoreMode::Ingress {
            return Err(Error::Authorization);
        }
        state.validate()?;
        Ok(state)
    }
}
impl Storage for EncryptedIngressStorage {
    fn load(&self) -> Result<Option<State>> {
        if self.failed {
            return Err(Error::Storage);
        }
        Ok(Some(self.state.clone()))
    }
    fn committed_view(&self) -> Option<State> {
        Some(self.state.clone())
    }
    fn suspend_storage(&mut self) -> Result<()> {
        self.backend.suspend_storage()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        if self.failed {
            return Err(Error::Storage);
        }
        let outer = self.backend.resume_storage()?.ok_or(Error::InvalidState)?;
        self.state = self.unseal(&outer)?;
        Ok(Some(self.state.clone()))
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        if self.failed {
            return Err(Error::Storage);
        }
        state.validate()?;
        if state.authorization != self.binding.digest() || state.mode != StoreMode::Ingress {
            return Err(Error::Authorization);
        }
        let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidState)?);
        if bytes.len().checked_add(256).ok_or(Error::Capacity)? > self.max_bytes {
            return Err(Error::Capacity);
        }
        let nonce = random_bytes::<12>()?;
        let cipher = Aes256Gcm::new_from_slice(&*self.key).map_err(|_| Error::InvalidState)?;
        let sealed = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &bytes,
                    aad: &self.aad(),
                },
            )
            .map_err(|_| Error::Storage)?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&sealed);
        let mut outer = State::empty(self.binding.digest())?;
        outer.mode = StoreMode::Ingress;
        outer
            .private_records
            .insert(BLOB.into(), SecretRecord::new(blob));
        if let Err(error) = self.backend.commit(&outer) {
            self.failed = true;
            return Err(error);
        }
        self.state = state.clone();
        Ok(())
    }
}
