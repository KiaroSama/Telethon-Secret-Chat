//! Explicit, offline rebinding of encrypted stores. Ordinary store opens never migrate.
//!
//! Quiesce all store users, close their handles, and back up the ciphertext before
//! migrating. Each call replaces one file; there is no transaction across stores.

use std::collections::BTreeSet;
use std::{fmt, path::Path};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Serialize, de::DeserializeOwned};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

use crate::custody::CustodyRecord;
use crate::errors::SecretChatError;
use crate::ingress::PersistedIngressState;
use crate::storage_backend::PersistedStoreSnapshot;
use crate::storage_file::{
    MAX_STORE_BYTES, absolute_store_path, atomic_replace, lock_store, read_store,
};
use crate::transport::compute_v2_authorization_identity;

const CHAT_MAGIC: &[u8; 8] = b"KSCSTO01";
const CUSTODY_MAGIC: &[u8; 8] = b"KSCCUS01";
const INGRESS_MAGIC: &[u8; 8] = b"KSCING01";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

type Result<T> = std::result::Result<T, SecretChatError>;

/// Caller attestation for an already verified grammers authorization.
///
/// The caller must independently establish that the exact historical v1 binding
/// and the current session identify the same authorization, and obtain the DC
/// and tag from that session's `Client::authorization_key_tag()`. The legacy
/// digest does not commit to this tag. These offline functions authenticate the
/// snapshot with its storage key and check its old binding; they cannot derive or
/// verify the relationship between the historical device hash and a supplied tag.
pub struct BindingOwnershipProof {
    pub user_id: i64,
    pub dc_id: i32,
    pub generation: String,
    pub auth_key_tag: [u8; 32],
    pub legacy_v1_binding: String,
}

impl Drop for BindingOwnershipProof {
    fn drop(&mut self) {
        self.auth_key_tag.zeroize();
    }
}

impl fmt::Debug for BindingOwnershipProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BindingOwnershipProof")
            .field("auth_key_tag", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl BindingOwnershipProof {
    /// Derive the same lowercase digest as the grammers transport.
    pub fn expected_v2_binding(&self) -> Result<String> {
        if self.user_id <= 0
            || self.dc_id <= 0
            || self.generation.trim().is_empty()
            || self.generation.len() > 128
        {
            return Err(corrupt(
                "invalid authorization identity for binding migration",
            ));
        }
        Ok(compute_v2_authorization_identity(
            self.user_id,
            self.auth_key_tag,
            self.dc_id,
            &self.generation,
        ))
    }
}

/// Reports only the binding digests and the number of records actually rewritten.
/// An already migrated file reports zero records. Ingress counts pending records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationSummary {
    pub migrated_records: usize,
    pub from_binding: String,
    pub to_binding: String,
    pub already_migrated: bool,
}

/// Rebind an existing `FileStorage` snapshot and its associated data.
/// `ProtectedFileStorage` has a different envelope and is not accepted here.
pub fn migrate_chat_store_v1_to_v2(
    path: &Path,
    key: [u8; 32],
    proof: &BindingOwnershipProof,
) -> Result<MigrationSummary> {
    migrate(path, key, proof, CHAT_MAGIC, |file, old, new| {
        let new_aad = [CHAT_MAGIC.as_slice(), new.as_bytes()].concat();
        if let Some(current) = file.decode::<PersistedStoreSnapshot>(&new_aad)? {
            current.validate(new)?;
            return Ok(None);
        }
        let old_aad = [CHAT_MAGIC.as_slice(), old.as_bytes()].concat();
        let mut state: PersistedStoreSnapshot = file.require_decode(&old_aad)?;
        state.validate(old)?;
        state.account_binding = new.to_owned();
        Ok(Some((state.chats.len(), file.encode(&state, &new_aad)?)))
    })
}

/// Rebind an existing `CustodyStore` file. The records are the only persisted
/// owner binding, so an empty file cannot establish ownership.
pub fn migrate_custody_store_v1_to_v2(
    path: &Path,
    key: [u8; 32],
    proof: &BindingOwnershipProof,
) -> Result<MigrationSummary> {
    migrate(path, key, proof, CUSTODY_MAGIC, |file, old, new| {
        let mut records: Vec<CustodyRecord> = file.require_decode(&[])?;
        let owner = records
            .first()
            .ok_or_else(|| corrupt("empty custody snapshot does not establish ownership"))?
            .owner_auth_id();
        let already_migrated = binding_matches(owner, old, new)?;
        let mut chat_ids = BTreeSet::new();
        for record in &records {
            record
                .validate_persistence()
                .map_err(|_| corrupt("invalid durable custody record"))?;
            if !chat_ids.insert(record.chat_id()) || record.owner_auth_id() != owner {
                return Err(corrupt("custody snapshot identities are inconsistent"));
            }
        }
        if already_migrated {
            return Ok(None);
        }
        for record in &mut records {
            record.owner_auth_id = new.to_owned();
        }
        Ok(Some((records.len(), file.encode(&records, &[])?)))
    })
}

/// Rebind an existing `EncryptedIngressStore` file without changing its replay
/// history or pending records.
pub fn migrate_ingress_store_v1_to_v2(
    path: &Path,
    key: [u8; 32],
    proof: &BindingOwnershipProof,
) -> Result<MigrationSummary> {
    migrate(path, key, proof, INGRESS_MAGIC, |file, old, new| {
        let mut state: PersistedIngressState = file.require_decode(&[])?;
        let already_migrated = binding_matches(&state.owner_auth_id, old, new)?;
        let count = state
            .validate_for_migration()
            .map_err(|_| corrupt("invalid durable ingress snapshot"))?;
        if already_migrated {
            return Ok(None);
        }
        state.owner_auth_id = new.to_owned();
        Ok(Some((count, file.encode(&state, &[])?)))
    })
}

fn migrate(
    path: &Path,
    key: [u8; 32],
    proof: &BindingOwnershipProof,
    magic: &[u8; 8],
    rebind: impl FnOnce(EncryptedSnapshot<'_>, &str, &str) -> Result<Option<(usize, Vec<u8>)>>,
) -> Result<MigrationSummary> {
    let key = Zeroizing::new(key);
    let to_binding = proof.expected_v2_binding()?;
    let old = &proof.legacy_v1_binding;
    if old.len() != 64
        || !old
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || old == &to_binding
    {
        return Err(corrupt("invalid legacy binding for migration"));
    }
    let path = absolute_store_path(path).map_err(|_| SecretChatError::StorageRequired)?;
    let _lock = lock_store(&path).map_err(|_| corrupt("store lock is unavailable or invalid"))?;
    let raw = read_store(&path)
        .map_err(|_| corrupt("failed to read the store for migration"))?
        .ok_or_else(|| corrupt("cannot migrate a missing store"))?;
    let header_len = magic.len() + NONCE_LEN;
    if raw.len() < header_len + TAG_LEN || !raw.starts_with(magic) {
        return Err(corrupt("invalid store envelope for migration"));
    }
    let nonce: [u8; NONCE_LEN] = raw[magic.len()..header_len]
        .try_into()
        .map_err(|_| corrupt("invalid store nonce for migration"))?;
    let file = EncryptedSnapshot {
        magic,
        nonce,
        ciphertext: &raw[header_len..],
        cipher: Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key)),
    };
    let replacement = rebind(file, old, &to_binding)?;
    let already_migrated = replacement.is_none();
    let mut migrated_records = 0;
    if let Some((count, envelope)) = replacement {
        atomic_replace(&path, &envelope).map_err(|failure| {
            if failure.replaced {
                corrupt("store was replaced but durability is uncertain; reopen before retrying")
            } else {
                corrupt("failed to commit store migration; original snapshot was preserved")
            }
        })?;
        migrated_records = count;
    }
    Ok(MigrationSummary {
        migrated_records,
        from_binding: old.clone(),
        to_binding,
        already_migrated,
    })
}

struct EncryptedSnapshot<'a> {
    magic: &'a [u8; 8],
    nonce: [u8; NONCE_LEN],
    ciphertext: &'a [u8],
    cipher: Aes256Gcm,
}

impl EncryptedSnapshot<'_> {
    fn decode<T: DeserializeOwned>(&self, aad: &[u8]) -> Result<Option<T>> {
        let Ok(plaintext) = self.cipher.decrypt(
            &Nonce::from(self.nonce),
            Payload {
                msg: self.ciphertext,
                aad,
            },
        ) else {
            return Ok(None);
        };
        let plaintext = Zeroizing::new(plaintext);
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|_| corrupt("store document failed migration schema decoding"))
    }

    fn require_decode<T: DeserializeOwned>(&self, aad: &[u8]) -> Result<T> {
        self.decode(aad)?
            .ok_or_else(|| corrupt("store authentication failed for migration"))
    }

    fn encode<T: Serialize>(&self, state: &T, aad: &[u8]) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(state).map_err(|_| corrupt("failed to serialize migrated store"))?,
        );
        if plaintext.len() > MAX_STORE_BYTES - self.magic.len() - NONCE_LEN - TAG_LEN {
            return Err(corrupt("migrated store exceeds its byte limit"));
        }
        let mut nonce = [0; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "OS randomness unavailable".into(),
        })?;
        let ciphertext = self
            .cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad,
                },
            )
            .map_err(|_| corrupt("failed to encrypt migrated store"))?;
        Ok([self.magic.as_slice(), &nonce, &ciphertext].concat())
    }
}

fn binding_matches(stored: &str, old: &str, new: &str) -> Result<bool> {
    if bool::from(stored.as_bytes().ct_eq(new.as_bytes())) {
        Ok(true)
    } else if bool::from(stored.as_bytes().ct_eq(old.as_bytes())) {
        Ok(false)
    } else {
        Err(corrupt(
            "store owner does not match either migration binding",
        ))
    }
}

fn corrupt(reason: &'static str) -> SecretChatError {
    SecretChatError::StoreCorrupt {
        chat_id: None,
        reason: reason.into(),
    }
}
