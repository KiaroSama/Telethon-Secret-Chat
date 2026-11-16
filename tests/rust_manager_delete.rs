#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    error::Error,
    manager::{Event, Options, SecretChatManager},
    storage::MemoryStorage,
};
use std::sync::{Arc, Mutex, atomic::Ordering};
use support::*;
async fn open() -> (
    Manager,
    Manager,
    Arc<Mutex<World>>,
    i32,
    Arc<Mutex<Vec<bool>>>,
) {
    let (a, b, world, id) = established().await;
    b.send_message(id, "so history is not empty", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    b.on("ChatClosedEvent", "closed", move |e| {
        if let Event::ChatClosed {
            history_deleted, ..
        } = e
        {
            sink.lock().unwrap().push(history_deleted);
        }
        Ok(())
    })
    .unwrap();
    (a, b, world, id, events)
}
fn absent(a: &Manager, id: i32) {
    assert!(!a.list().unwrap().iter().any(|c| c.id == id));
    assert!(matches!(a.status(id), Err(Error::UnknownChat { .. })));
    assert!(a.read_history(id, 10).is_err());
    assert!(a.pending_relay_deliveries(id).is_err());
}
#[tokio::test]
async fn delete_both_sides_removes_both_chat_states_and_reports_history() {
    let (a, b, world, id, events) = open().await;
    assert!(a.delete_secret_chat_both_sides(id).await.unwrap());
    pump(&a, &b, &world).await;
    absent(&a, id);
    absent(&b, id);
    assert_eq!(world.lock().unwrap().discards, [(id, true)]);
    assert_eq!(*events.lock().unwrap(), [true]);
}
#[tokio::test]
async fn delete_both_closed_tombstone_reports_remote_refusal() {
    let (a, b, world, id, _) = open().await;
    a.close(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert!(!a.delete_secret_chat_both_sides(id).await.unwrap());
    assert_eq!(world.lock().unwrap().discards, [(id, false), (id, true)]);
    absent(&a, id);
}
#[tokio::test]
async fn delete_one_side_leaves_peer_tombstone() {
    let (a, b, world, id, events) = open().await;
    a.delete_secret_chat(id).await.unwrap();
    pump(&a, &b, &world).await;
    absent(&a, id);
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(*events.lock().unwrap(), [false]);
    assert_eq!(world.lock().unwrap().discards, [(id, false)]);
}
#[tokio::test]
async fn delete_one_side_closed_tombstone_sends_nothing_more() {
    let (a, _b, world, id, _) = open().await;
    a.close(id).await.unwrap();
    a.delete_secret_chat(id).await.unwrap();
    absent(&a, id);
    assert_eq!(world.lock().unwrap().discards, [(id, false)]);
}
async fn repeated(both: bool) {
    let (a, _b, world, id, _) = open().await;
    let remove = |id| {
        let a = a.clone();
        async move {
            if both {
                a.delete_secret_chat_both_sides(id).await.map(|_| ())
            } else {
                a.delete_secret_chat(id).await
            }
        }
    };
    assert!(matches!(remove(999).await, Err(Error::UnknownChat { .. })));
    remove(id).await.unwrap();
    let sent = world.lock().unwrap().discards.len();
    assert!(matches!(remove(id).await, Err(Error::UnknownChat { .. })));
    assert_eq!(world.lock().unwrap().discards.len(), sent);
}
#[tokio::test]
async fn unknown_and_repeated_local_delete_are_refused() {
    repeated(false).await;
}
#[tokio::test]
async fn unknown_and_repeated_both_sides_delete_are_refused() {
    repeated(true).await;
}
#[tokio::test]
async fn failed_storage_removal_keeps_closed_record_installed() {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let fail = Arc::clone(&store.failures);
    let a = SecretChatManager::new(aa, store, "11".repeat(32), Options::default()).unwrap();
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
    a.close(id).await.unwrap();
    fail.store(1, Ordering::Release);
    assert!(matches!(
        a.delete_secret_chat(id).await,
        Err(Error::Storage)
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
}
#[tokio::test]
async fn delete_after_shutdown_is_refused_without_removing_ready_chat() {
    let (a, _b, _world, id, _) = open().await;
    a.stop().await.unwrap();
    assert!(matches!(
        a.delete_secret_chat(id).await,
        Err(Error::ManagerStopping)
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
}
#[tokio::test]
async fn peer_history_delete_removes_existing_tombstone() {
    let (a, b, world, id, events) = open().await;
    b.close(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    a.delete_secret_chat_both_sides(id).await.unwrap();
    pump(&a, &b, &world).await;
    absent(&b, id);
    assert_eq!(*events.lock().unwrap(), [false, true]);
}
#[tokio::test]
async fn plain_peer_close_keeps_tombstone_and_reports_no_history_delete() {
    let (a, b, world, id, events) = open().await;
    a.close(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(*events.lock().unwrap(), [false]);
}
