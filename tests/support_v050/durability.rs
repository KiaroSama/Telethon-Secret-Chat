#![forbid(unsafe_code)]

use std::path::Path;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use kiaro_secret_chat::{
    binding::AuthorizationBinding,
    binding_migration::{LegacyAuthorizationBinding, migrate_v1_json_store_to_v2},
    chat::Chat,
    crypto::SharedKey,
    ingress::EncryptedIngressStorage,
    protocol::Protocol,
    storage::{FileStorage, MemoryStorage, SecretRecord, State, Storage, StoreMode},
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn native_state(authorization: String) -> TestResult<State> {
    let mut protocol = Protocol::open(MemoryStorage::default(), authorization.clone())?;
    protocol.start()?;
    let mut chat = Chat::for_role(7, 8, 9, true);
    chat.adopt_key(SharedKey::from_slice(&[0x57; 256])?)?;
    protocol.insert_chat(chat)?;
    protocol.send_message(7, "retained request".into(), None, None)?;
    let mut state = State::empty(authorization)?;
    state.chats.insert(7, protocol.chat_record(7)?);
    state.validate()?;
    Ok(state)
}

fn duplicate_chat_json(state: &State) -> TestResult<String> {
    let original = serde_json::to_string(&state.chats[&7])?;
    let mut replacement = state.chats[&7].clone();
    replacement.outbox.clear();
    replacement.validate()?;
    let replacement = serde_json::to_string(&replacement)?;
    let needle = format!("\"chats\":{{\"7\":{original}}}");
    let duplicate = format!("\"chats\":{{\"7\":{original},\"7\":{replacement}}}");
    let valid = serde_json::to_string(state)?;
    assert!(valid.contains(&needle));
    Ok(valid.replacen(&needle, &duplicate, 1))
}

fn write_native(path: &Path, state: &State, bytes: &str) -> TestResult {
    let mut store = FileStorage::open(path)?;
    store.commit(state)?;
    drop(store);
    // Keep the private permissions from a real native store publication.
    std::fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn native_reopen_refuses_duplicate_chat_identity_without_dropping_pending_outbox() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native-duplicate-chat.json");
    let state = native_state("11".repeat(32))?;
    let encoded = duplicate_chat_json(&state)?;
    write_native(&path, &state, &encoded)?;
    let result = FileStorage::open(&path)?.load();
    assert!(
        result.is_err(),
        "native reopen accepted two chat identities and silently replaced the pending outbox"
    );
    assert_eq!(std::fs::read(&path)?, encoded.as_bytes());
    Ok(())
}

#[test]
fn native_reopen_refuses_duplicate_outbox_sequence_instead_of_clearing_pending() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native-duplicate-outbox.json");
    let state = native_state("11".repeat(32))?;
    let (sequence, original) = state.chats[&7]
        .outbox
        .first_key_value()
        .ok_or("no outbox")?;
    let mut replacement = original.clone();
    replacement.pending = false;
    let original = serde_json::to_string(original)?;
    let replacement = serde_json::to_string(&replacement)?;
    let valid = serde_json::to_string(&state)?;
    let needle = format!("\"outbox\":{{\"{sequence}\":{original}}}");
    let duplicate =
        format!("\"outbox\":{{\"{sequence}\":{original},\"{sequence}\":{replacement}}}");
    assert!(valid.contains(&needle));
    let encoded = valid.replacen(&needle, &duplicate, 1);
    write_native(&path, &state, &encoded)?;
    assert!(
        FileStorage::open(&path)?.load().is_err(),
        "native reopen normalized a duplicate sequence to a nonpending transmission"
    );
    assert_eq!(std::fs::read(&path)?, encoded.as_bytes());
    Ok(())
}

#[test]
fn historical_migration_refuses_duplicate_chat_identity_and_preserves_original_file() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("historical-duplicate.json");
    let old = LegacyAuthorizationBinding::from_parts(101, 202, 2, "schema-recheck")?;
    let new = AuthorizationBinding::v2(101, 2, [0x65; 32], "schema-recheck")?;
    let state = native_state(old.fingerprint())?;
    let encoded = duplicate_chat_json(&state)?;
    write_native(&path, &state, &encoded)?;
    assert!(
        migrate_v1_json_store_to_v2(&path, &old, &new).is_err(),
        "migration accepted an ambiguous chat map and published a normalized snapshot losing its pending frame"
    );
    assert_eq!(std::fs::read(&path)?, encoded.as_bytes());
    Ok(())
}

#[test]
fn native_reopen_and_historical_migration_preserve_valid_unique_state() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("historical-valid.json");
    let old = LegacyAuthorizationBinding::from_parts(101, 202, 2, "schema-recheck")?;
    let new = AuthorizationBinding::v2(101, 2, [0x65; 32], "schema-recheck")?;
    let state = native_state(old.fingerprint())?;
    let mut store = FileStorage::open(&path)?;
    store.commit(&state)?;
    drop(store);
    let reopened = FileStorage::open(&path)?;
    let original = reopened.load()?.ok_or("missing native state")?;
    assert!(serde_json::to_value(&original)? == serde_json::to_value(&state)?);
    drop(reopened);
    let report = migrate_v1_json_store_to_v2(&path, &old, &new)?;
    assert_eq!(report.migrated_records, 1);
    let reopened = FileStorage::open(&path)?;
    let migrated = reopened.load()?.ok_or("missing migrated state")?;
    let mut expected = serde_json::to_value(state)?;
    expected["authorization"] = json!(new.fingerprint());
    assert!(serde_json::to_value(migrated)? == expected);
    Ok(())
}

#[test]
fn authenticated_native_ingress_reopen_refuses_duplicate_inner_chat_identity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native-encrypted-duplicate.json");
    let binding = AuthorizationBinding::v2(101, 2, [0x65; 32], "schema-recheck")?;
    let key = [0x68; 32];
    let mut inner = native_state(binding.fingerprint())?;
    inner.mode = StoreMode::Ingress;
    let mut encrypted = EncryptedIngressStorage::open(&path, key, binding.clone(), 1024 * 1024)?;
    encrypted.commit(&inner)?;
    drop(encrypted);
    let valid = EncryptedIngressStorage::open(&path, key, binding.clone(), 1024 * 1024)?;
    assert!(
        serde_json::to_value(valid.load()?.ok_or("missing encrypted state")?)?
            == serde_json::to_value(&inner)?
    );
    drop(valid);

    let nonce = [0x79; 12];
    let aad = [
        b"KIARO-INGRESS-RUST-1\0".as_slice(),
        binding.fingerprint().as_bytes(),
    ]
    .concat();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid fixture key")?;
    let ciphertext = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: duplicate_chat_json(&inner)?.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| "could not encrypt fixture")?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    let mut outer = State::empty(binding.fingerprint())?;
    outer.mode = StoreMode::Ingress;
    outer
        .private_records
        .insert("aead/ingress-v1".into(), SecretRecord::new(blob));
    let mut store = FileStorage::open(&path)?;
    store.commit(&outer)?;
    drop(store);
    let before = std::fs::read(&path)?;
    assert!(
        EncryptedIngressStorage::open(&path, key, binding, 1024 * 1024).is_err(),
        "authenticated ingress accepted an ambiguous inner chat map and lost its pending frame"
    );
    assert_eq!(std::fs::read(&path)?, before);
    Ok(())
}

#[test]
fn native_store_rejects_an_existing_data_file_as_its_lock_sidecar() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native.json");
    let sidecar = directory.path().join("native.json.lock");
    let state = State::empty("11".repeat(32))?;
    let mut other_store = FileStorage::open(&sidecar)?;
    other_store.commit(&state)?;
    let before = std::fs::read(&sidecar)?;
    assert!(
        FileStorage::open(&path).is_err(),
        "native store accepted another data file as its replacement-sensitive lock inode"
    );
    assert_eq!(std::fs::read(&sidecar)?, before);
    assert!(!path.exists());
    assert!(other_store.load()?.is_some());
    Ok(())
}

#[test]
fn native_sidecar_alias_cannot_admit_two_owners_after_an_atomic_replacement() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native.json");
    let sidecar = directory.path().join("native.json.lock");
    let state = State::empty("11".repeat(32))?;
    let mut original = FileStorage::open(&path)?;
    original.commit(&state)?;
    assert!(FileStorage::open(&path).is_err());
    if let Ok(mut alias) = FileStorage::open(&sidecar) {
        // Some platforms reject replacement of a locked file; either refusal is safe.
        if alias.commit(&state).is_ok() {
            assert!(
                FileStorage::open(&path).is_err(),
                "replacing the sidecar through a second store admitted another owner while the original was still active"
            );
        }
    }
    let mut updated = state;
    updated
        .settings
        .insert("original_owner".into(), json!(true));
    original.commit(&updated)?;
    assert!(
        original.load()?.ok_or("missing original state")?.settings["original_owner"] == json!(true)
    );
    Ok(())
}
