#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    binding::AuthorizationBinding, ingress::EncryptedIngressStorage, storage::Storage,
};
fn binding(id: i64) -> AuthorizationBinding {
    AuthorizationBinding::from_parts(123, id, 2, "fixture-generation".into()).unwrap()
}
#[test]
fn protected_checkpoint_roundtrip_and_exclusive_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.bin");
    let key = [7; 32];
    let mut store = EncryptedIngressStorage::open(&path, key, binding(456), 1024 * 1024).unwrap();
    let mut state = store.load().unwrap().unwrap();
    state.settings.insert(
        "checkpoint".into(),
        serde_json::json!({"qts":40,"marker":"plaintext fixture sentinel"}),
    );
    store.commit(&state).unwrap();
    assert!(
        !std::fs::read(&path)
            .unwrap()
            .windows(26)
            .any(|w| w == b"plaintext fixture sentinel")
    );
    assert!(EncryptedIngressStorage::open(&path, key, binding(456), 1024 * 1024).is_err());
    drop(store);
    let mut restored =
        EncryptedIngressStorage::open(&path, key, binding(456), 1024 * 1024).unwrap();
    assert_eq!(
        restored.load().unwrap().unwrap().settings["checkpoint"]["qts"],
        40
    );
    let before = std::fs::read(&path).unwrap();
    let mut too_big = restored.load().unwrap().unwrap();
    too_big
        .settings
        .insert("large".into(), serde_json::json!("x".repeat(1024 * 1024)));
    assert!(restored.commit(&too_big).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        restored.load().unwrap().unwrap().settings["checkpoint"]["qts"],
        40
    );
    drop(restored);
    let large_path = dir.path().join("large-custody.bin");
    let budget = 8 * 1024 * 1024;
    let mut large = EncryptedIngressStorage::open(&large_path, key, binding(456), budget).unwrap();
    let mut state = large.load().unwrap().unwrap();
    state
        .settings
        .insert("large".into(), "x".repeat(5 * 1024 * 1024).into());
    large.commit(&state).unwrap();
    drop(large);
    let restored = EncryptedIngressStorage::open(&large_path, key, binding(456), budget).unwrap();
    assert_eq!(
        restored.load().unwrap().unwrap().settings["large"]
            .as_str()
            .unwrap()
            .len(),
        5 * 1024 * 1024
    );
}
fn refused(change: u8) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.bin");
    let key = [7; 32];
    let store = EncryptedIngressStorage::open(&path, key, binding(456), 1024 * 1024).unwrap();
    drop(store);
    if change == 2 {
        let mut outer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let blob = outer["private_records"]["aead/ingress-v1"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut bytes = hex::decode(blob).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        outer["private_records"]["aead/ingress-v1"] = hex::encode(bytes).into();
        std::fs::write(&path, serde_json::to_vec(&outer).unwrap()).unwrap();
    }
    assert!(
        EncryptedIngressStorage::open(
            &path,
            if change == 0 { [8; 32] } else { key },
            binding(if change == 1 { 457 } else { 456 }),
            1024 * 1024
        )
        .is_err()
    );
}
#[test]
fn protected_store_refuses_wrong_key() {
    refused(0)
}
#[test]
fn protected_store_refuses_wrong_binding() {
    refused(1)
}
#[test]
fn protected_store_refuses_corrupt_ciphertext() {
    refused(2)
}
#[test]
fn established_aead_matches_published_nist_empty_plaintext_vector() {
    use aes_gcm::{Aes128Gcm, KeyInit, Nonce, aead::Aead};
    let cipher = Aes128Gcm::new_from_slice(&[0; 16]).unwrap();
    assert_eq!(
        hex::encode(cipher.encrypt(&Nonce::from([0; 12]), &[][..]).unwrap()),
        "58e2fccefa7e3061367f1d57a4e7455a"
    );
}
