#![allow(clippy::unwrap_used)]
#[path = "support_manager/bounded.rs"]
mod bounded;
#[path = "support_manager/mod.rs"]
mod support;
use bounded::*;
use kiaro_secret_chat::{
    custody::CustodyStorage,
    enums::{DecryptedMessage, DecryptedMessageAction},
    error::Error,
    manager::{Event, Options, SecretChatManager},
    storage::FileStorage,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use support::*;
fn reopen(path: &std::path::Path, world: &Arc<std::sync::Mutex<World>>) -> Bounded<FileStorage> {
    SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(world),
        },
        CustodyStorage::new(FileStorage::open(path).unwrap(), "11".repeat(32), limits()).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap()
}
#[tokio::test]
async fn failed_callback_keeps_original_receipt_through_file_reconstruction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let (a, b, w, id) = pair(
        FileStorage::open(&path).unwrap(),
        limits(),
        Options::default(),
    )
    .await;
    let called = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&called);
    a.on("MessageReceived", "failure", move |event| {
        if let Event::MessageReceived(_) = event {
            seen.fetch_add(1, Ordering::AcqRel);
        }
        Err(Error::Media)
    })
    .unwrap();
    let random = b
        .send_message(id, "relay fixture", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(called.load(Ordering::Acquire), 1);
    let held = a.pending_relay_deliveries(id).unwrap();
    assert_eq!(held.len(), 1);
    let body = kiaro_secret_chat::framing::unwrap(&held[0].message.body).unwrap();
    assert_eq!(body.message.random_id(), random);
    assert!(a.acknowledge_delivery(id, -1).is_err());
    assert!(a.acknowledge_delivery(id, held[0].seq_no ^ 1).is_err());
    a.stop().await.unwrap();
    let resumed = reopen(&path, &w);
    resumed.start().await.unwrap();
    let after = resumed.pending_relay_deliveries(id).unwrap();
    assert_eq!(after[0].seq_no, held[0].seq_no);
    assert_eq!(after[0].message.body, held[0].message.body);
    resumed.acknowledge_delivery(id, held[0].seq_no).unwrap();
    assert!(resumed.pending_relay_deliveries(id).unwrap().is_empty());
    for effect in &w.lock().unwrap().sent {
        let layer = kiaro_secret_chat::framing::unwrap(&effect.outgoing.body).unwrap();
        assert!(
            !matches!(layer.message,DecryptedMessage::Service(s)if matches!(s.action,DecryptedMessageAction::ReadMessages(_)))
        );
    }
    resumed.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn local_protocol_close_preserves_consumer_obligation_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let (a, b, w, id) = pair(
        FileStorage::open(&path).unwrap(),
        limits(),
        Options::default(),
    )
    .await;
    b.send_message(id, "retained relay", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    let held = a.pending_relay_deliveries(id).unwrap();
    a.close(id).await.unwrap();
    assert!(w.lock().unwrap().discards.is_empty());
    a.stop().await.unwrap();
    let resumed = reopen(&path, &w);
    resumed.start().await.unwrap();
    let after = resumed.pending_relay_deliveries(id).unwrap();
    assert_eq!(after[0].message.body, held[0].message.body);
    resumed.acknowledge_delivery(id, held[0].seq_no).unwrap();
    assert!(resumed.pending_relay_deliveries(id).unwrap().is_empty());
    resumed.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn failed_ack_commit_preserves_original_consumer_obligation() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), limits(), Options::default()).await;
    b.send_message(id, "ack fixture", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    let held = a.pending_relay_deliveries(id).unwrap();
    store.failures.store(1, Ordering::Release);
    assert!(a.acknowledge_delivery(id, held[0].seq_no).is_err());
    let after = a.pending_relay_deliveries(id).unwrap();
    assert_eq!(after[0].message.body, held[0].message.body);
    assert_eq!(after[0].seq_no, held[0].seq_no);
    a.acknowledge_delivery(id, held[0].seq_no).unwrap();
    assert!(a.pending_relay_deliveries(id).unwrap().is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn owed_consumer_receipt_does_not_block_ttl_protocol() {
    let (a, b, w, id) = pair(TestStorage::default(), limits(), Options::default()).await;
    b.send_message(id, "held relay", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    b.set_ttl(id, 31).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(a.status(id).unwrap().ttl, 31);
    assert_eq!(a.pending_relay_deliveries(id).unwrap().len(), 1);
    let reply = a
        .send_message(id, "reply fixture", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(b.read_history(id, 1).unwrap()[0].ttl, 31);
    a.delete_messages(id, vec![reply]).await.unwrap();
    pump(&a, &b, &w).await;
    assert!(b.read_history(id, 10).unwrap().is_empty());
    assert_eq!(a.pending_relay_deliveries(id).unwrap().len(), 1);
    a.flush_history(id).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(a.pending_relay_deliveries(id).unwrap().len(), 1);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
