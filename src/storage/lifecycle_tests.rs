#![allow(clippy::unwrap_used)]
use super::*;
#[cfg(unix)]
fn exact_state(actual: Option<State>, expected: &State) {
    assert!(serde_json::to_vec(&actual.unwrap()).unwrap() == serde_json::to_vec(expected).unwrap());
}
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
    // A child between fork and exec can retain this open-file description even
    // with CLOEXEC. A safe duplicate exercises the same Unix lock lifetime.
    #[cfg(unix)]
    let inherited_lock = store._lock.as_ref().unwrap().try_clone().unwrap();
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
    #[cfg(unix)]
    {
        assert!(inherited_lock.metadata().unwrap().is_file());
        drop(inherited_lock);

        let mut stopped = FileStorage::open(&one).unwrap();
        let inherited_lock = stopped._lock.as_ref().unwrap().try_clone().unwrap();
        stopped.suspend_storage().unwrap();
        let mut successor = FileStorage::open(&one).unwrap();
        exact_state(successor.load().unwrap(), &state);
        successor.suspend_storage().unwrap();
        exact_state(stopped.resume_storage().unwrap(), &state);
        assert!(FileStorage::open(&one).is_err());
        stopped.suspend_storage().unwrap();
        let successor = FileStorage::open(&one).unwrap();
        drop(stopped);
        assert!(
            FileStorage::open(&one).is_err(),
            "dropping a stopped old owner must not release its successor"
        );
        exact_state(successor.load().unwrap(), &state);
        assert!(inherited_lock.metadata().unwrap().is_file());
        drop(inherited_lock);
        drop(successor);

        // A forked copy must not explicitly unlock its parent's shared open
        // description. Only the process that acquired custody may release it.
        let mut foreign = FileStorage::open(&one).unwrap();
        let parent_lock = foreign._lock.as_ref().unwrap().try_clone().unwrap();
        foreign.owner_pid = std::process::id().wrapping_add(1);
        assert!(foreign.suspend_storage().is_err());
        drop(foreign);
        assert!(FileStorage::open(&one).is_err());
        parent_lock.unlock().unwrap();
        exact_state(FileStorage::open(&one).unwrap().load().unwrap(), &state);
        assert!(parent_lock.metadata().unwrap().is_file());
    }
}
