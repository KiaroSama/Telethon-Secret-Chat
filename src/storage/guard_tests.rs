#![allow(clippy::unwrap_used)]
use super::contract_tests::{incoming, record};
use super::*;
use crate::test_support::logging;
#[test]
fn recovery_window_edge_is_accepted_and_next_refused() {
    let mut chat = record(1);
    chat.in_seq_no = 0;
    assert!(
        crate::sequence::preflight(
            &chat,
            &crate::framing::unwrap(&incoming(1000).body).unwrap()
        )
        .unwrap()
    );
    assert!(
        crate::sequence::preflight(
            &chat,
            &crate::framing::unwrap(&incoming(1001).body).unwrap()
        )
        .is_err()
    );
}
#[test]
fn full_gap_queue_refuses_more_custody() {
    let mut chat = record(1);
    for seq in 1001..=2000 {
        chat.gaps.insert(seq, incoming(seq));
    }
    assert!(
        crate::sequence::preflight(&chat, &crate::framing::unwrap(&incoming(5).body).unwrap())
            .is_err()
    );
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix symlink privilege contract")]
fn symlinked_store_refuses_without_touching_target() {
    super::foundation::symlinked_state_is_refused();
    super::foundation::symlinked_lock_is_refused();
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.json");
        let link = dir.path().join("link.json");
        let mut store = FileStorage::open(&target).unwrap();
        store
            .commit(&State::empty("11".repeat(32)).unwrap())
            .unwrap();
        let before = fs::read(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(FileStorage::open(&link).is_err());
        assert_eq!(fs::read(target).unwrap(), before);
    }
}
fn invalid_document(bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    store
        .commit(&State::empty("11".repeat(32)).unwrap())
        .unwrap();
    fs::write(&path, bytes).unwrap();
    assert!(matches!(store.load(), Err(Error::InvalidState)));
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn non_utf8_document_refused() {
    super::foundation::malformed_store_refuses_start();
    super::foundation::directory_cannot_be_used_as_state();
    super::foundation::invalid_authorization_binding_is_refused();
    super::foundation::authorization_mismatch_is_refused();
    invalid_document(b"\xff\xfe garbage");
}
#[test]
fn list_document_refused() {
    invalid_document(b"[]");
}
#[test]
fn missing_store_fields_refused() {
    invalid_document(br#"{"chats":{},"out":{}}"#);
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix directory fsync contract")]
fn ordinary_directory_sync_failure_warns_after_whole_replace() {
    #[cfg(unix)]
    {
        let capture = logging::Capture::default();
        let logs = std::sync::Arc::clone(&capture.0);
        let _guard = tracing::subscriber::set_default(capture);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut store = FileStorage::open(&path).unwrap();
        let mut state = State::empty("11".repeat(32)).unwrap();
        state.chats.insert(1, record(1));
        store.fail_next_commit(CommitFault::DirectorySync);
        store.commit(&state).unwrap();
        drop(store);
        assert!(
            FileStorage::open(&path)
                .unwrap()
                .load()
                .unwrap()
                .unwrap()
                .chats
                .contains_key(&1)
        );
        assert!(
            logs.lock()
                .unwrap()
                .join("\n")
                .contains("Directory durability could not be confirmed")
        );
    }
}
