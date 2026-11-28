use std::path::PathBuf;

use kiaro_secret_chat::custody::{
    CustodyBackend, CustodyMode, CustodyRecord, CustodyStore, StoreCustodyLimits,
};
use kiaro_secret_chat::errors::SecretChatError;

#[test]
fn custody_modes_enforce_explicit_selection_and_reject_cwd_fallback() {
    // Explicit relative path without an explicit root/parent must be rejected (no CWD fallback).
    let relative_err =
        CustodyStore::open_durable(PathBuf::from("relative-only.json"), [0x22; 32]).unwrap_err();
    assert!(matches!(relative_err, SecretChatError::StorageRequired));

    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("custody.enc");
    let mut store =
        CustodyStore::open_durable(&store_path, [0x22; 32]).expect("open explicit durable store");

    let record = CustodyRecord::new(
        101,
        "auth-owner-1",
        CustodyMode::Durable,
        CustodyBackend::EncryptedFile,
        StoreCustodyLimits::default(),
    );
    store.save_record(&record).expect("save durable record");

    let loaded = store.load_record(101, "auth-owner-1").expect("load record");
    assert_eq!(loaded.chat_id(), 101);
    assert_eq!(loaded.mode(), CustodyMode::Durable);
    assert!(!loaded.is_paused());

    // Wrong authorization holder cannot read stored custody state; diagnostic stays secret-free.
    let err = store.load_record(101, "auth-other").unwrap_err();
    assert!(matches!(err, SecretChatError::StoreCorrupt { .. }));
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains("22222222"));
}

#[test]
fn pause_resume_corruption_bounds_and_cleanup_preserve_unrelated_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unrelated = dir.path().join("unrelated-user-file.txt");
    std::fs::write(&unrelated, b"keep-me").expect("write unrelated");

    let store_path = dir.path().join("bounded.enc");
    let mut store = CustodyStore::open_durable(&store_path, [0x44; 32]).expect("open");

    let limits = StoreCustodyLimits {
        max_retained_messages: 2,
        max_stored_bytes: 128,
    };
    let mut rec = CustodyRecord::new(
        202,
        "auth-owner-1",
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        limits,
    );

    // Pause blocks new message ingestion until resumed.
    rec.pause("operator pause");
    assert!(rec.is_paused());
    assert!(rec.push_payload(b"blocked").is_err());

    rec.resume();
    assert!(!rec.is_paused());
    rec.push_payload(b"msg-1").expect("push 1");
    rec.push_payload(b"msg-2").expect("push 2");
    // Capacity refusal preserves the two existing obligations and pauses custody.
    assert!(rec.push_payload(b"msg-3").is_err());
    assert!(rec.is_paused());
    assert_eq!(rec.retained_count(), 2);

    // Oversized single payload is refused without mutating retained state.
    assert!(rec.push_payload(&[0xaa; 200]).is_err());
    assert_eq!(rec.retained_count(), 2);

    store.save_record(&rec).expect("save");

    // Corrupting the on-disk file fails closed without leaking key bytes.
    let pristine = std::fs::read(&store_path).expect("read");
    let mut disk = pristine.clone();
    disk[0] ^= 0xff;
    std::fs::write(&store_path, disk).expect("corrupt");
    assert!(matches!(
        store.load_record(202, "auth-owner-1"),
        Err(SecretChatError::StoreCorrupt { .. })
    ));

    // Unauthenticated data is preserved for explicit caller recovery.
    assert!(store.cleanup_owned_files().is_err());
    assert!(store_path.exists());
    std::fs::write(&store_path, pristine).expect("restore authenticated snapshot");

    // Cleanup removes only an authenticated store and never erases unrelated files.
    store.cleanup_owned_files().expect("cleanup");
    assert!(!store_path.exists());
    assert_eq!(
        std::fs::read(&unrelated).expect("unrelated intact"),
        b"keep-me"
    );
}
