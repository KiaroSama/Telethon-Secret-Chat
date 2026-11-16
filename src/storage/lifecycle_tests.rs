#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn opening_a_competing_backend_preserves_the_active_commit() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.json");
    let two = dir.path().join("two.json");
    let mut store = FileStorage::open(&one).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, super::contract_tests::record(7));
    let mut reached = false;
    store
        .commit_with_sync(&state, |file| {
            reached = true;
            assert!(file.metadata()?.len() > 0);
            let staging = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "tmp"))
                .collect::<Vec<_>>();
            assert_eq!(staging.len(), 1);
            let competitor = FileStorage::open(&two).unwrap();
            assert!(
                staging[0].exists(),
                "opening another state file must not remove an active commit"
            );
            assert!(competitor.load().unwrap().is_none());
            file.sync_all()
        })
        .unwrap();
    assert!(reached);
    assert!(store.load().unwrap().unwrap().chats.contains_key(&7));
    drop(store);
    assert!(
        FileStorage::open(&one)
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .chats
            .contains_key(&7)
    );
}
