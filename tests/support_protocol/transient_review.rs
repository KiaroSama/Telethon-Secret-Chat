#![allow(clippy::unwrap_used)]
use super::*;
use crate::{crypto::SharedKey, protocol::Protocol, storage::MemoryStorage};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Default)]
struct Failing {
    store: MemoryStorage,
    fail: Arc<AtomicBool>,
}
impl Storage for Failing {
    fn load(&self) -> Result<Option<State>> {
        self.store.load()
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            Err(Error::Storage)
        } else {
            self.store.commit(state)
        }
    }
}
fn initialized() -> TransientStorage<Failing> {
    let mut store = TransientStorage::new(
        Failing::default(),
        "11".repeat(32),
        TransientLimits {
            chats: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let mut source = Protocol::open(MemoryStorage::default(), "11".repeat(32)).unwrap();
    source.start().unwrap();
    for id in [7, 8] {
        let mut chat = crate::chat::Chat::for_role(id, 9, 10, true);
        chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
            .unwrap();
        source.insert_chat(chat).unwrap();
        source
            .send_message(id, "retained".into(), None, None)
            .unwrap();
    }
    let mut state = store.state.clone();
    for id in [7, 8] {
        state.chats.insert(id, source.chat_record(id).unwrap());
    }
    store.commit(&state).unwrap();
    store
}
pub(crate) fn verify_refusals() {
    let mut store = initialized();
    store.admit(Some(7), Operation::Accept, 0).unwrap();
    store.admit(None, Operation::Create, 0).unwrap();
    let mut creation = store.state.clone();
    creation
        .chats
        .insert(9, crate::chat::Chat::for_role(9, 10, 11, true));
    assert_eq!(store.commit(&creation), Err(Error::Capacity));
    assert_eq!(
        store.state.chats[&7].state,
        ChatState::Ready,
        "unbound creation cannot suspend a previous admission"
    );
    assert!(!store.state.chats[&7].outbox.is_empty());

    let mut store = initialized();
    for ((id, _), created) in &mut store.created {
        if *id == 7 {
            *created = now().unwrap() - 600_000;
        }
    }
    store.protected.fail.store(true, Ordering::SeqCst);
    let first = store.expire_runtime().unwrap();
    assert_eq!(first.chats, [7]);
    assert!(first.persistence_failed);
    assert!(
        store.next_runtime_expiry_ms().unwrap().is_some(),
        "refused participation must still schedule later volatile cleanup"
    );
    for ((id, _), created) in &mut store.created {
        if *id == 8 {
            *created = now().unwrap() - 600_000;
        }
    }
    let later = store.expire_runtime().unwrap();
    assert_eq!(later.chats, [8]);
    assert!(later.persistence_failed);
    assert!(
        store
            .state
            .chats
            .values()
            .all(|chat| chat.state == ChatState::Suspended && chat.outbox.is_empty())
    );
}
