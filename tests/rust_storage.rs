#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    error::Error,
    storage::{FileStorage, MemoryStorage, State, Storage},
};
fn state() -> State {
    State::empty("11".repeat(32)).unwrap()
}
#[test]
fn empty_memory_store_is_explicitly_volatile() {
    assert!(MemoryStorage::default().load().unwrap().is_none());
}
#[test]
fn memory_returns_detached_snapshot() {
    let mut store = MemoryStorage::default();
    store.commit(&state()).unwrap();
    let mut loaded = store.load().unwrap().unwrap();
    loaded.settings.insert("x".into(), true.into());
    assert!(store.load().unwrap().unwrap().settings.is_empty());
}
#[test]
fn memory_refuses_invalid_commit_without_losing_previous_state() {
    let mut store = MemoryStorage::default();
    store.commit(&state()).unwrap();
    let mut corrupt = state();
    corrupt.version = 100;
    assert!(store.commit(&corrupt).is_err());
    assert_eq!(store.load().unwrap().unwrap().version, 1);
}
#[test]
fn file_state_survives_reconstruction() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    {
        let mut store = FileStorage::open(&path).unwrap();
        let mut state = state();
        state
            .settings
            .insert("text".into(), "✓ فارسی 日本語".into());
        store.commit(&state).unwrap();
    }
    let store = FileStorage::open(&path).unwrap();
    assert_eq!(
        store.load().unwrap().unwrap().settings["text"],
        "✓ فارسی 日本語"
    );
}
#[test]
fn second_writer_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let _first = FileStorage::open(&path).unwrap();
    assert!(FileStorage::open(&path).is_err());
}
#[test]
fn missing_parent_is_not_created_implicitly() {
    let directory = tempfile::tempdir().unwrap();
    assert!(FileStorage::open(directory.path().join("missing/state.json")).is_err());
    assert!(!directory.path().join("missing").exists());
}
#[test]
fn malformed_store_refuses_start() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    std::fs::write(&path, b"{broken").unwrap();
    let store = FileStorage::open(&path).unwrap();
    assert!(store.load().is_err());
}
#[test]
fn directory_cannot_be_used_as_state() {
    let directory = tempfile::tempdir().unwrap();
    assert!(FileStorage::open(directory.path()).is_err());
}
#[test]
fn invalid_authorization_binding_is_refused() {
    for value in ["".into(), "x".repeat(64), "aa".repeat(31)] {
        assert!(matches!(State::empty(value), Err(Error::Authorization)));
    }
}
#[test]
fn authorization_mismatch_is_refused() {
    let mut store = MemoryStorage::default();
    store.commit(&state()).unwrap();
    assert!(matches!(
        kiaro_secret_chat::protocol::Protocol::open(store, "22".repeat(32)),
        Err(Error::Authorization)
    ));
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix filesystem permissions and symlink contract")]
fn file_state_is_owner_only() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut store = FileStorage::open(&path).unwrap();
        store.commit(&state()).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix filesystem permissions and symlink contract")]
fn symlinked_state_is_refused() {
    #[cfg(unix)]
    {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, b"original").unwrap();
        let link = directory.path().join("state.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(FileStorage::open(&link).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"original");
    }
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix filesystem permissions and symlink contract")]
fn symlinked_lock_is_refused() {
    #[cfg(unix)]
    {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, b"original").unwrap();
        let link = directory.path().join("state.json.lock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(FileStorage::open(directory.path().join("state.json")).is_err());
    }
}
#[test]
#[cfg_attr(not(unix), ignore = "Unix filesystem permissions and symlink contract")]
fn permissive_existing_state_is_refused() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut store = FileStorage::open(&path).unwrap();
        store.commit(&state()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.load().is_err());
    }
}
