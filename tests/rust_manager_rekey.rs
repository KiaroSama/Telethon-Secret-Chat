#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    enums, framing,
    manager::{Options, RpcFailure, SecretChatManager},
    protocol::Protocol,
    storage::MemoryStorage,
};
use std::sync::{Arc, Mutex};
use support::*;
async fn threshold(
    left_pending: bool,
) -> (
    SecretChatManager<TestStorage, Fake>,
    Manager,
    Arc<Mutex<World>>,
    i32,
) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let transport = aa.clone();
    let store = TestStorage::default();
    let a = SecretChatManager::new(aa, store.clone(), "11".repeat(32), Options::default()).unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    if left_pending {
        world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
        assert!(
            a.send_message(id, "left pending", None, None)
                .await
                .is_err()
        );
    }
    drop(a);
    let mut protocol = Protocol::open(store.clone(), "11".repeat(32)).unwrap();
    protocol.start().unwrap();
    protocol
        .mutate_chat(id, |chat| {
            chat.messages_since_rekey = 101;
            Ok(())
        })
        .unwrap();
    protocol.stop().unwrap();
    drop(protocol);
    let a = SecretChatManager::new(transport, store, "11".repeat(32), Options::default()).unwrap();
    (a, b, world, id)
}
#[tokio::test]
async fn documented_trigger_rekeys_automatically_and_peers_agree() {
    let (a, b, world, id) = threshold(false).await;
    let before = a.status(id).unwrap().key_fingerprint;
    a.send_message(id, "the one that notices", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_ne!(a.status(id).unwrap().key_fingerprint, before);
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
    assert_eq!(
        b.read_history(id, 1).unwrap()[0].text,
        "the one that notices"
    );
}
#[tokio::test]
async fn concurrent_service_sends_start_only_one_exchange_after_retry() {
    let (a, _b, world, id) = threshold(true).await;
    let (one, two) = tokio::join!(a.set_typing(id, None), a.mark_read(id, vec![1]));
    one.unwrap();
    two.unwrap();
    let requests=world.lock().unwrap().sent.iter().filter(|effect|matches!(framing::unwrap(&effect.outgoing.body).unwrap().message,enums::DecryptedMessage::Service(s)if matches!(s.action,enums::DecryptedMessageAction::RequestKey(_)))).count();
    assert_eq!(requests, 1);
}
