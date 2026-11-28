//! Independent audit cases for aliasing, uniqueness and atomic rollback.
#![allow(clippy::unwrap_used)]
use super::{contract_tests::*, *};
use crate::protocol::Protocol;
fn protocol(store: Backend) -> Protocol<Backend> {
    let mut p = Protocol::open(store, "11".repeat(32)).unwrap();
    p.start().unwrap();
    p
}
macro_rules! paired {
    ($memory:ident,$file:ident,$body:expr) => {
        #[test]
        fn $memory() {
            let mut fixture = Fixture::new(false);
            ($body)(&mut fixture);
        }
        #[test]
        fn $file() {
            let mut fixture = Fixture::new(true);
            ($body)(&mut fixture);
        }
    };
}
paired!(
    detached_record_and_retention_memory,
    detached_record_and_retention_file,
    |f: &mut Fixture| {
        super::foundation::empty_memory_store_is_explicitly_volatile();
        super::foundation::memory_returns_detached_snapshot();
        super::foundation::memory_refuses_invalid_commit_without_losing_previous_state();
        let mut state = queued();
        state
            .settings
            .insert("nested".into(), serde_json::json!({"items":[1]}));
        f.commit(&state);
        state.settings.get_mut("nested").unwrap()["items"]
            .as_array_mut()
            .unwrap()
            .push(2.into());
        state
            .chats
            .get_mut(&1)
            .unwrap()
            .outbox
            .get_mut(&3)
            .unwrap()
            .body
            .push(55);
        let mut loaded = f.state();
        loaded.settings.get_mut("nested").unwrap()["items"]
            .as_array_mut()
            .unwrap()
            .push(3.into());
        loaded
            .chats
            .get_mut(&1)
            .unwrap()
            .outbox
            .get_mut(&3)
            .unwrap()
            .body
            .push(77);
        assert_eq!(
            f.state().settings["nested"]["items"],
            serde_json::json!([1])
        );
        assert_eq!(f.state().chats[&1].outbox[&3].body, b"public-1");
    }
);
paired!(
    single_outgoing_identity_memory,
    single_outgoing_identity_file,
    |f: &mut Fixture| {
        let mut state = queued();
        f.commit(&state);
        let old = state.chats[&1].outbox[&3].clone();
        state
            .chats
            .get_mut(&1)
            .unwrap()
            .outbox
            .get_mut(&3)
            .unwrap()
            .pending = false;
        f.commit(&state);
        let state = f.state();
        let new = &state.chats[&1].outbox[&3];
        assert!(!new.pending);
        assert_eq!(state.chats[&1].outbox.len(), 3);
        assert_eq!((new.random_id, &new.frame), (old.random_id, &old.frame));
    }
);
paired!(
    duplicate_gap_memory,
    duplicate_gap_file,
    |f: &mut Fixture| {
        let mut state = queued();
        state.chats.get_mut(&1).unwrap().gaps.clear();
        for _ in 0..20 {
            crate::sequence::accept(state.chats.get_mut(&1).unwrap(), incoming(3)).unwrap();
        }
        let first = state.chats[&1].gaps[&3].body.clone();
        let mut replacement = incoming(3);
        replacement.file = Some(b"different attachment".to_vec());
        crate::sequence::accept(state.chats.get_mut(&1).unwrap(), replacement).unwrap();
        f.commit(&state);
        assert_eq!(f.state().chats[&1].gaps.len(), 1);
        assert_eq!(f.state().chats[&1].gaps[&3].body, first);
        assert!(f.state().chats[&1].gaps[&3].file.is_none());
    }
);
fn file_mutation(operation: u8) {
    let mut fixture = Fixture::new(true);
    fixture.commit(&queued());
    let before = fs::read(fixture.dir.path().join("state.json")).unwrap();
    fixture.fail();
    let mut p = protocol(fixture.store);
    let original = serde_json::to_vec(&p.chat_record(1).unwrap()).unwrap();
    let result = p.transaction(|state| {
        match operation {
            0 => {
                state.chats.remove(&1);
            }
            1 => {
                state
                    .chats
                    .get_mut(&1)
                    .unwrap()
                    .outbox
                    .insert(9, outgoing(4));
            }
            2 => {
                state
                    .chats
                    .get_mut(&1)
                    .unwrap()
                    .outbox
                    .retain(|seq, _| *seq > 3);
            }
            3 => {
                state.chats.get_mut(&1).unwrap().gaps.insert(4, incoming(4));
            }
            4 => {
                state.chats.get_mut(&1).unwrap().gaps.clear();
            }
            _ => unreachable!(),
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        serde_json::to_vec(&p.chat_record(1).unwrap()).unwrap(),
        original
    );
    assert_eq!(
        fs::read(fixture.dir.path().join("state.json")).unwrap(),
        before
    );
    // A failure before replacement retains a usable backend and exact queues.
    p.mutate_chat(1, |chat| {
        chat.ttl = 1;
        Ok(())
    })
    .unwrap();
    drop(p);
    let reopened = FileStorage::open(fixture.dir.path().join("state.json"))
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(reopened.chats[&1].ttl, 1);
    assert_eq!(reopened.chats[&1].outbox.len(), 3);
    assert_eq!(reopened.chats[&1].gaps.len(), 3);
}
#[test]
fn delete_failure_rollback() {
    file_mutation(0);
}
#[test]
fn queue_out_failure_rollback() {
    file_mutation(1);
}
#[test]
fn drop_out_failure_rollback() {
    file_mutation(2);
}
#[test]
fn queue_in_failure_rollback() {
    file_mutation(3);
}
#[test]
fn take_in_failure_rollback() {
    file_mutation(4);
}
paired!(
    record_and_queues_atomic_memory,
    record_and_queues_atomic_file,
    |f: &mut Fixture| {
        let mut state = queued();
        state.chats.get_mut(&1).unwrap().outbox.clear();
        state.chats.get_mut(&1).unwrap().gaps.clear();
        f.commit(&state);
        let backend = std::mem::replace(
            &mut f.store,
            Backend::Memory(MemoryStorage::default(), false),
        );
        let mut p = protocol(backend);
        let result = p.transaction(|state| -> Result<()> {
            let chat = state.chats.get_mut(&1).unwrap();
            chat.ttl = 9;
            chat.outbox.insert(3, outgoing(1));
            chat.gaps.insert(2, incoming(2));
            Err(Error::Storage)
        });
        assert!(result.is_err());
        let loaded = p.chat_record(1).unwrap();
        assert_eq!(loaded.ttl, 0);
        assert!(loaded.outbox.is_empty() && loaded.gaps.is_empty());
    }
);
#[test]
fn multiple_operations_make_one_backend_commit() {
    struct Counted {
        backend: Backend,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Storage for Counted {
        fn load(&self) -> Result<Option<State>> {
            self.backend.load()
        }
        fn commit(&mut self, state: &State) -> Result<()> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.backend.commit(state)
        }
    }
    let mut f = Fixture::new(true);
    f.commit(&queued());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut p = Protocol::open(
        Counted {
            backend: f.store,
            calls: std::sync::Arc::clone(&calls),
        },
        "11".repeat(32),
    )
    .unwrap();
    p.start().unwrap();
    p.transaction(|state| {
        let chat = state.chats.get_mut(&1).unwrap();
        chat.ttl = 1;
        chat.outbox.insert(9, outgoing(4));
        Ok(())
    })
    .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(p);
    let state = FileStorage::open(f.dir.path().join("state.json"))
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(state.chats[&1].ttl, 1);
    assert!(state.chats[&1].outbox.contains_key(&9));
}
