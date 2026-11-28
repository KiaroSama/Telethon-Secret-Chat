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
        let backend = open_backend(path.as_ref(), max_bytes)?;
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
            store.state = unseal(&outer, &store.key, &store.binding.fingerprint(), max_bytes)?;
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
}

pub(crate) fn open_backend(path: &Path, max_bytes: usize) -> Result<FileStorage> {
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
    FileStorage::open(absolute)
}

fn aad(binding: &str) -> Vec<u8> {
    [MAGIC, binding.as_bytes()].concat()
}

pub(crate) fn unseal(
    outer: &State,
    key: &[u8; 32],
    binding: &str,
    max_bytes: usize,
) -> Result<State> {
    if outer.authorization != binding
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
    if blob.len() < 28 || blob.len() > max_bytes + 28 {
        return Err(Error::InvalidState);
    }
    let nonce: [u8; 12] = blob[..12].try_into().map_err(|_| Error::InvalidState)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::InvalidState)?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &blob[12..],
                    aad: &aad(binding),
                },
            )
            .map_err(|_| Error::Authorization)?,
    );
    if plaintext.len() + 256 > max_bytes {
        return Err(Error::Capacity);
    }
    let state: State = serde_json::from_slice(&plaintext).map_err(|_| Error::InvalidState)?;
    if state.authorization != binding || state.mode != StoreMode::Ingress {
        return Err(Error::Authorization);
    }
    state.validate()?;
    Ok(state)
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
        self.state = unseal(
            &outer,
            &self.key,
            &self.binding.fingerprint(),
            self.max_bytes,
        )?;
        Ok(Some(self.state.clone()))
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        if self.failed {
            return Err(Error::Storage);
        }
        let outer = seal(
            state,
            &self.key,
            &self.binding.fingerprint(),
            self.max_bytes,
        )?;
        if let Err(error) = self.backend.commit(&outer) {
            self.failed = true;
            return Err(error);
        }
        self.state = state.clone();
        Ok(())
    }
}

pub(crate) fn seal(
    state: &State,
    key: &[u8; 32],
    binding: &str,
    max_bytes: usize,
) -> Result<State> {
    state.validate()?;
    if state.authorization != binding || state.mode != StoreMode::Ingress {
        return Err(Error::Authorization);
    }
    let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidState)?);
    if bytes.len().checked_add(256).ok_or(Error::Capacity)? > max_bytes {
        return Err(Error::Capacity);
    }
    let nonce = random_bytes::<12>()?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::InvalidState)?;
    let sealed = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &bytes,
                aad: &aad(binding),
            },
        )
        .map_err(|_| Error::Storage)?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&sealed);
    let mut outer = State::empty(binding.to_owned())?;
    outer.mode = StoreMode::Ingress;
    outer
        .private_records
        .insert(BLOB.into(), SecretRecord::new(blob));
    Ok(outer)
}
