#![allow(clippy::unwrap_used)]
use super::support::*;
use kiaro_secret_chat::{
    chat::{Chat, ChatState, DhRecord},
    crypto::SharedKey,
    manager::{Options, SecretChatManager},
    storage::State,
};
use std::sync::{Arc, Mutex};
pub fn chat() -> Chat {
    let mut chat = Chat::new(7, 88, 2000, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: prime(),
    });
    chat.capability_layer = 144;
    chat.wrapper_layer = 144;
    chat.rekeyed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    chat
}
pub type Seeded = SecretChatManager<TestStorage, Fake>;
pub async fn ready(options: Options) -> (Seeded, TestStorage, Arc<Mutex<World>>, i32) {
    ready_chat(chat(), options).await
}
pub async fn ready_chat(
    chat: Chat,
    options: Options,
) -> (Seeded, TestStorage, Arc<Mutex<World>>, i32) {
    let value = construct(chat, options);
    value.0.start().await.unwrap();
    value
}
pub fn construct(chat: Chat, options: Options) -> (Seeded, TestStorage, Arc<Mutex<World>>, i32) {
    let id = chat.id;
    let store = TestStorage::default();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(id, chat);
    *store.state.lock().unwrap() = Some(state);
    let (transport, _) = Fake::pair();
    let world = Arc::clone(&transport.world);
    world
        .lock()
        .unwrap()
        .requests
        .insert(id, (1000, 2000, vec![]));
    let manager =
        SecretChatManager::new(transport, store.clone(), "11".repeat(32), options).unwrap();
    (manager, store, world, id)
}
