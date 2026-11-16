//! Both shipped ordinary backends obey the same whole-state transaction contract.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    Serializable,
    chat::{Chat, ChatState, Envelope, Exchange, Outgoing},
    crypto::SharedKey,
};

pub(super) enum Backend {
    Memory(MemoryStorage, bool),
    File(FileStorage),
}
impl Storage for Backend {
    fn load(&self) -> Result<Option<State>> {
        match self {
            Self::Memory(store, _) => store.load(),
            Self::File(store) => store.load(),
        }
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        match self {
            Self::Memory(store, fault) => {
                if std::mem::take(fault) {
                    return Err(Error::Storage);
                }
                store.commit(state)
            }
            Self::File(store) => store.commit(state),
        }
    }
}
pub(super) struct Fixture {
    pub(super) dir: tempfile::TempDir,
    pub(super) store: Backend,
}
impl Fixture {
    pub(super) fn new(file: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = if file {
            Backend::File(FileStorage::open(dir.path().join("state.json")).unwrap())
        } else {
            Backend::Memory(MemoryStorage::default(), false)
        };
        Self { dir, store }
    }
    pub(super) fn state(&self) -> State {
        self.store
            .load()
            .unwrap()
            .unwrap_or_else(|| State::empty("11".repeat(32)).unwrap())
    }
    pub(super) fn commit(&mut self, state: &State) {
        self.store.commit(state).unwrap();
    }
    pub(super) fn fail(&mut self) {
        match &mut self.store {
            Backend::Memory(_, fault) => *fault = true,
            Backend::File(store) => store.fail_next_commit(CommitFault::BeforeReplace),
        }
    }
    pub(super) fn reopen(&mut self) {
        assert!(matches!(&self.store, Backend::File(_)));
        let old = std::mem::replace(
            &mut self.store,
            Backend::Memory(MemoryStorage::default(), false),
        );
        drop(old);
        self.store = Backend::File(FileStorage::open(self.dir.path().join("state.json")).unwrap());
    }
}
pub(super) fn record(id: i32) -> Chat {
    let mut chat = Chat::new(id, 999, 555, true, ChatState::Ready);
    chat.key = Some(SharedKey::from_slice(&[0xaa; 256]).unwrap());
    chat.dh = Some(crate::chat::DhRecord {
        g: 2,
        prime: hex::decode(include_str!("../../tests/fixtures/dh-prime.hex").trim()).unwrap(),
    });
    chat.out_seq_no = 20;
    chat
}
pub(super) fn incoming(raw: i32) -> Envelope {
    Envelope {
        body: crate::framing::wrap(
            crate::types::DecryptedMessage {
                no_webpage: false,
                silent: false,
                random_id: i64::from(raw + 1),
                ttl: 0,
                message: format!("public-{raw}"),
                media: None,
                entities: None,
                via_bot_name: None,
                reply_to_random_id: None,
                grouped_id: None,
            }
            .into(),
            144,
            1,
            raw * 2,
        )
        .unwrap()
        .to_bytes(),
        file: None,
        date: 1,
    }
}
pub(super) fn outgoing(raw: i32) -> Outgoing {
    Outgoing {
        seq_no: raw * 2 + 1,
        random_id: i64::from(raw + 100),
        frame: vec![raw as u8; 40],
        body: format!("public-{raw}").into_bytes(),
        failures: 0,
        service: false,
        file: None,
        pending: true,
        obligation_generation: 1,
    }
}
pub(super) fn queued() -> State {
    let mut state = State::empty("11".repeat(32)).unwrap();
    let mut chat = record(1);
    for raw in [1, 2, 3] {
        chat.outbox.insert(raw * 2 + 1, outgoing(raw));
        chat.gaps.insert(raw, incoming(raw));
    }
    state.chats.insert(1, chat);
    state
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
paired!(saved_chat_memory, saved_chat_file, |f: &mut Fixture| {
    let mut state = f.state();
    state.chats.insert(1, record(1));
    f.commit(&state);
    let loaded = f.state();
    let key = loaded.chats[&1].key.as_ref().unwrap();
    assert_eq!(key.expose(), &[0xaa; 256]);
    assert_eq!(
        key.fingerprint(),
        SharedKey::from_slice(&[0xaa; 256]).unwrap().fingerprint()
    );
});
paired!(unknown_chat_memory, unknown_chat_file, |f: &mut Fixture| {
    assert!(!f.state().chats.contains_key(&404));
});
paired!(replace_chat_memory, replace_chat_file, |f: &mut Fixture| {
    let mut state = f.state();
    state.chats.insert(1, record(1));
    f.commit(&state);
    state.chats.get_mut(&1).unwrap().key = Some(SharedKey::from_slice(&[0xbb; 256]).unwrap());
    f.commit(&state);
    assert_eq!(f.state().chats.len(), 1);
    assert_eq!(
        f.state().chats[&1].key.as_ref().unwrap().expose(),
        &[0xbb; 256]
    );
});
paired!(
    delete_chat_queues_memory,
    delete_chat_queues_file,
    |f: &mut Fixture| {
        let mut state = queued();
        f.commit(&state);
        state.chats.remove(&1);
        f.commit(&state);
        assert!(!f.state().chats.contains_key(&1));
    }
);
paired!(list_chats_memory, list_chats_file, |f: &mut Fixture| {
    let mut state = f.state();
    for id in [1, 2] {
        state.chats.insert(id, record(id));
    }
    f.commit(&state);
    assert_eq!(
        f.state().chats.keys().copied().collect::<Vec<_>>(),
        vec![1, 2]
    );
});
paired!(
    interrupted_key_save_memory,
    interrupted_key_save_file,
    |f: &mut Fixture| {
        let mut state = queued();
        f.commit(&state);
        let fingerprint = state.chats[&1].key.as_ref().unwrap().fingerprint();
        state.chats.get_mut(&1).unwrap().key = Some(SharedKey::from_slice(&[0xbb; 256]).unwrap());
        f.fail();
        assert!(f.store.commit(&state).is_err());
        assert_eq!(
            f.state().chats[&1].key.as_ref().unwrap().expose(),
            &[0xaa; 256]
        );
        assert_eq!(
            f.state().chats[&1].key.as_ref().unwrap().fingerprint(),
            fingerprint
        );
    }
);
paired!(rekey_pair_memory, rekey_pair_file, |f: &mut Fixture| {
    let mut state = queued();
    let chat = state.chats.get_mut(&1).unwrap();
    chat.state = ChatState::Rekeying;
    chat.dh = Some(crate::chat::DhRecord {
        g: 2,
        prime: hex::decode(include_str!("../../tests/fixtures/dh-prime.hex").trim()).unwrap(),
    });
    chat.exchange = Some(Exchange::Accepted {
        id: 77,
        key: SharedKey::from_slice(&[0xcc; 256]).unwrap(),
    });
    f.commit(&state);
    let loaded = f.state();
    assert_eq!(
        loaded.chats[&1].key.as_ref().unwrap().expose(),
        &[0xaa; 256]
    );
    assert!(
        matches!(loaded.chats[&1].exchange.as_ref(),Some(Exchange::Accepted{id:77,key}) if key.expose()==&[0xcc;256])
    );
});
paired!(
    counters_with_key_memory,
    counters_with_key_file,
    |f: &mut Fixture| {
        let mut state = queued();
        let chat = state.chats.get_mut(&1).unwrap();
        chat.in_seq_no = 12;
        chat.out_seq_no = 11;
        chat.gaps.clear();
        for raw in [13, 14, 15] {
            chat.gaps.insert(raw, incoming(raw));
        }
        f.commit(&state);
        let loaded = f.state();
        assert_eq!(
            (loaded.chats[&1].in_seq_no, loaded.chats[&1].out_seq_no),
            (12, 11)
        );
        assert!(loaded.chats[&1].key.is_some());
    }
);
paired!(ordered_gap_memory, ordered_gap_file, |f: &mut Fixture| {
    let mut state = queued();
    let chat = state.chats.get_mut(&1).unwrap();
    chat.gaps.clear();
    for raw in [4, 2, 3] {
        chat.gaps.insert(raw, incoming(raw));
    }
    f.commit(&state);
    assert_eq!(
        f.state().chats[&1].gaps.keys().copied().collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
});
paired!(drain_gap_memory, drain_gap_file, |f: &mut Fixture| {
    let mut state = queued();
    f.commit(&state);
    let first = std::mem::take(&mut state.chats.get_mut(&1).unwrap().gaps);
    assert_eq!(first.len(), 3);
    f.commit(&state);
    assert!(f.state().chats[&1].gaps.is_empty());
});
paired!(
    ack_retention_memory,
    ack_retention_file,
    |f: &mut Fixture| {
        let mut state = queued();
        let chat = state.chats.get_mut(&1).unwrap();
        for raw in [0, 4] {
            chat.outbox.insert(raw * 2 + 1, outgoing(raw));
        }
        f.commit(&state);
        state
            .chats
            .get_mut(&1)
            .unwrap()
            .outbox
            .retain(|seq, _| *seq > 5);
        f.commit(&state);
        assert_eq!(
            f.state().chats[&1]
                .outbox
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![7, 9]
        );
    }
);
paired!(
    nested_failure_memory,
    nested_failure_file,
    |f: &mut Fixture| {
        f.commit(&State::empty("11".repeat(32)).unwrap());
        let store = std::mem::replace(
            &mut f.store,
            Backend::Memory(MemoryStorage::default(), false),
        );
        let mut protocol = crate::protocol::Protocol::open(store, "11".repeat(32)).unwrap();
        protocol.start().unwrap();
        let result = protocol.transaction(|draft| {
            draft.chats.insert(1, record(1));
            fn fail_inner(draft: &mut State) -> Result<()> {
                draft
                    .chats
                    .get_mut(&1)
                    .unwrap()
                    .outbox
                    .insert(3, outgoing(1));
                Err(Error::Storage)
            }
            fail_inner(draft)?;
            Ok(())
        });
        assert!(result.is_err());
        assert!(protocol.chat_records().is_empty());
    }
);
#[test]
#[cfg_attr(not(unix), ignore = "Unix filesystem permissions contract")]
fn file_backend_is_owner_only() {
    super::foundation::file_state_is_owner_only();
    super::foundation::permissive_existing_state_is_refused();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut f = Fixture::new(true);
        f.commit(&queued());
        assert_eq!(
            fs::metadata(f.dir.path().join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn both_queues_reconstruct() {
    super::foundation::file_state_survives_reconstruction();
    super::foundation::second_writer_is_refused();
    super::foundation::missing_parent_is_created_for_persistent_state();
    let mut f = Fixture::new(true);
    f.commit(&queued());
    f.reopen();
    let loaded = f.state();
    assert_eq!(
        loaded.chats[&1].gaps.keys().copied().collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        loaded.chats[&1].outbox.keys().copied().collect::<Vec<_>>(),
        vec![3, 5, 7]
    );
}
#[test]
fn gap_drain_persists() {
    let mut f = Fixture::new(true);
    let mut state = queued();
    f.commit(&state);
    state.chats.get_mut(&1).unwrap().gaps.clear();
    f.commit(&state);
    f.reopen();
    assert!(f.state().chats[&1].gaps.is_empty());
}
#[test]
fn orphan_temporary_file_removed_without_touching_other_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let prefix = temporary_prefix(&path).unwrap();
    let leftover = dir
        .path()
        .join(format!("{prefix}{}-Ab123456789Z.tmp", std::process::id()));
    fs::write(&leftover, b"public incomplete fixture").unwrap();
    let unrelated = dir.path().join("other.tmp");
    fs::write(&unrelated, b"unrelated").unwrap();
    let other = dir.path().join(format!(
        "{}{}-Ab123456789Z.tmp",
        temporary_prefix(&dir.path().join("other.json")).unwrap(),
        std::process::id()
    ));
    fs::write(&other, b"other state owner").unwrap();
    let _store = FileStorage::open(&path).unwrap();
    assert!(!leftover.exists());
    assert!(unrelated.exists() && other.exists());
}
#[test]
fn interrupted_replace_leaves_no_temporary_payload() {
    let mut f = Fixture::new(true);
    f.commit(&queued());
    let before = fs::read(f.dir.path().join("state.json")).unwrap();
    f.fail();
    assert!(f.store.commit(&queued()).is_err());
    assert_eq!(fs::read(f.dir.path().join("state.json")).unwrap(), before);
    assert!(!fs::read_dir(f.dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".secret-chat-store-")
    }));
}
#[test]
fn partial_gap_drain_commits_once_and_retains_later_gap() {
    let mut f = Fixture::new(true);
    let mut state = queued();
    let chat = state.chats.get_mut(&1).unwrap();
    chat.gaps.clear();
    for raw in 2..40 {
        chat.gaps.insert(raw, incoming(raw));
    }
    f.commit(&state);
    struct Counted {
        backend: Backend,
        count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Storage for Counted {
        fn load(&self) -> Result<Option<State>> {
            self.backend.load()
        }
        fn commit(&mut self, state: &State) -> Result<()> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.backend.commit(state)
        }
    }
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let backend = std::mem::replace(
        &mut f.store,
        Backend::Memory(MemoryStorage::default(), false),
    );
    let mut p = crate::protocol::Protocol::open(
        Counted {
            backend,
            count: std::sync::Arc::clone(&count),
        },
        "11".repeat(32),
    )
    .unwrap();
    p.start().unwrap();
    let frame = crate::crypto::encrypt_frame(
        &SharedKey::from_slice(&[0xaa; 256]).unwrap(),
        &incoming(0).body,
        crate::crypto::Direction::Responder,
    )
    .unwrap();
    p.receive(1, &frame, None, 1).unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(p);
    f.store = Backend::File(FileStorage::open(f.dir.path().join("state.json")).unwrap());
    assert_eq!(
        f.state().chats[&1].gaps.keys().copied().collect::<Vec<_>>(),
        (2..40).collect::<Vec<_>>()
    );
}
