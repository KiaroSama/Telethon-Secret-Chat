#![allow(clippy::unwrap_used)]
use super::*;
use crate::{chat::Chat, crypto::SharedKey, protocol::Protocol, storage::MemoryStorage};
fn initialized(mode: StoreMode) -> CustodyStorage<MemoryStorage> {
    let mut backend = MemoryStorage::default();
    let mut empty = State::empty("11".repeat(32)).unwrap();
    empty.mode = mode;
    backend.commit(&empty).unwrap();
    let mut limits = CustodyLimits::new(32, 1024 * 1024, 1.0).unwrap();
    limits.frame_bytes = 8192;
    let mut store = CustodyStorage::new(backend, "11".repeat(32), limits).unwrap();
    let mut protocol = Protocol::open(MemoryStorage::default(), "11".repeat(32)).unwrap();
    protocol.start().unwrap();
    let mut chat = Chat::for_role(7, 8, 9, true);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    protocol.insert_chat(chat).unwrap();
    protocol
        .send_message(7, "retained".into(), None, None)
        .unwrap();
    let mut state = store.state.clone();
    state.chats.insert(7, protocol.chat_record(7).unwrap());
    store.commit(&state).unwrap();
    store
}
pub(crate) fn verify_refusal_ownership() {
    for mode in [StoreMode::Bounded, StoreMode::Ingress] {
        let mut store = initialized(mode);
        store.admit(Some(7), Operation::Receive, 0).unwrap();
        for age in store.controls.get_mut(&7).unwrap().ages.values_mut() {
            *age = now().unwrap() - 5000;
        }
        let mut completion = store.state.clone();
        completion
            .chats
            .get_mut(&7)
            .unwrap()
            .outbox
            .values_mut()
            .next()
            .unwrap()
            .pending = false;
        assert_eq!(store.commit(&completion), Err(Error::Paused));
        assert!(
            store.status(7).unwrap().resumable,
            "a previous receive admission cannot own a send completion"
        );
        let mut settled = store.state.clone();
        settled.chats.get_mut(&7).unwrap().outbox.clear();
        store.commit(&settled).unwrap();
        store.resume(7).unwrap();
    }
    let mut store = initialized(StoreMode::Ingress);
    assert_eq!(
        store.admit(Some(7), Operation::Receive, 8193),
        Err(Error::Capacity)
    );
    assert!(
        store.status(7).unwrap().resumable,
        "journal-backed capacity refusal retains input"
    );
    store.resume(7).unwrap();
    for age in store.controls.get_mut(&7).unwrap().ages.values_mut() {
        *age = now().unwrap() - 5000;
    }
    assert_eq!(
        store.admit(Some(7), Operation::Receive, 0),
        Err(Error::Paused)
    );
    assert!(
        store.status(7).unwrap().resumable,
        "journal-backed age refusal retains input"
    );
}
