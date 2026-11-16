#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    custody::{CustodyLimits, CustodyStorage, TransientLimits, TransientStorage},
    error::Error,
    manager::{Event, Options, RpcFailure, SecretChatManager, SendFileOptions},
    media::FileSource,
    storage::MemoryStorage,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::*;
type BoundedManager = SecretChatManager<CustodyStorage<TestStorage>, Fake>;
type ProtectedManager = SecretChatManager<TransientStorage<TestStorage>, Fake>;
async fn bounded_pair(
    operation: f64,
    tasks: usize,
) -> (BoundedManager, Manager, TestStorage, Arc<Mutex<World>>, i32) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let mut limits = CustodyLimits::new(100, 1_000_000, 86400.0).unwrap();
    limits.operation_seconds = operation;
    limits.tasks = tasks;
    let a = SecretChatManager::new(
        aa,
        CustodyStorage::new(store.clone(), "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, store, world, id)
}
async fn protected_pair(
    operation: f64,
    tasks: usize,
) -> (
    ProtectedManager,
    Manager,
    TestStorage,
    Arc<Mutex<World>>,
    i32,
) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let limits = TransientLimits {
        operation_seconds: operation,
        tasks,
        ..Default::default()
    };
    let a = SecretChatManager::new(
        aa,
        TransientStorage::new(store.clone(), "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, store, world, id)
}
async fn soon<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
#[tokio::test]
async fn bounded_send_deadline_pauses_and_preserves_original_pending_identity() {
    refused_receive_task_permit_marks_continuity_lost().await;
    bounded_callback_capacity_keeps_both_consumer_obligations().await;
    cancelling_bounded_send_preserves_retryable_identity_without_pausing().await;
    let (a, _b, store, world, id) = bounded_pair(0.04, 2).await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(gate);
    let error = a
        .send_message(id, "deadline", None, None)
        .await
        .unwrap_err();
    let Error::SendPending {
        chat_id,
        random_id,
        cause,
    } = error
    else {
        panic!("expected retained pending identity");
    };
    assert_eq!(chat_id, id);
    assert_eq!(
        random_id,
        world
            .lock()
            .unwrap()
            .sent
            .last()
            .unwrap()
            .outgoing
            .random_id
    );
    assert_eq!(cause.name(), "CancelledError");
    assert!(a.status(id).unwrap().paused);
    let state = store.state.lock().unwrap();
    assert!(
        state.as_ref().unwrap().chats[&id]
            .outbox
            .values()
            .any(|r| r.pending)
    );
    assert!(state.as_ref().unwrap().chats[&id].key.is_some());
    assert!(world.lock().unwrap().discards.is_empty());
}
async fn cancelling_bounded_send_preserves_retryable_identity_without_pausing() {
    let (a, _b, _store, world, id) = bounded_pair(2.0, 2).await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let task = tokio::spawn(async move { manager.send_message(id, "cancel", None, None).await });
    soon(gate.entered.notified()).await;
    let before = world.lock().unwrap().sent.last().unwrap().clone();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!a.status(id).unwrap().paused);
    world.lock().unwrap().send_gate = None;
    a.retry_pending(id).await.unwrap();
    assert_eq!(
        world.lock().unwrap().sent.last().unwrap().outgoing.frame,
        before.outgoing.frame
    );
}
async fn bounded_callback_capacity_keeps_both_consumer_obligations() {
    let (a, b, _store, world, id) = bounded_pair(2.0, 2).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    let release = Arc::new(tokio::sync::Notify::new());
    let wait = Arc::clone(&release);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let delivered = Arc::clone(&seen);
    a.on_async("MessageReceived", "held", move |event| {
        let signal = Arc::clone(&signal);
        let wait = Arc::clone(&wait);
        let delivered = Arc::clone(&delivered);
        async move {
            if let Event::MessageReceived(message) = event {
                delivered.lock().unwrap().push(message.text.clone());
            }
            signal.notify_one();
            wait.notified().await;
            Ok(())
        }
    })
    .unwrap();
    b.send_message(id, "first", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    soon(entered.notified()).await;
    b.send_message(id, "second", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(a.active_handler_tasks(), 1);
    assert_eq!(a.pending_relay_deliveries(id).unwrap().len(), 2);
    assert!(a.read_history(id, 100).unwrap().is_empty());
    release.notify_one();
    a.settle().await.unwrap();
    a.resume_chat(id).await.unwrap();
    soon(entered.notified()).await;
    assert_eq!(*seen.lock().unwrap(), ["first", "second"]);
    assert_eq!(a.pending_relay_deliveries(id).unwrap().len(), 2);
    release.notify_one();
    a.settle().await.unwrap();
    a.stop().await.unwrap();
}
async fn refused_receive_task_permit_marks_continuity_lost() {
    let (a, b, store, world, id) = bounded_pair(2.0, 2).await;
    let second = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(second).await.unwrap();
    pump(&a, &b, &world).await;
    b.send_message(id, "unprocessed received frame", None, None)
        .await
        .unwrap();
    let incoming = world
        .lock()
        .unwrap()
        .updates
        .get_mut(&1000)
        .unwrap()
        .pop_front()
        .unwrap();
    let before = store.state.lock().unwrap().as_ref().unwrap().chats[&id].in_seq_no;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let first = a.clone();
    let held =
        tokio::spawn(async move { first.send_message(id, "held task one", None, None).await });
    soon(gate.entered.notified()).await;
    let next = a.clone();
    let other =
        tokio::spawn(async move { next.send_message(second, "held task two", None, None).await });
    soon(gate.entered.notified()).await;
    a.handle_update(&incoming).await;
    assert!(
        !a.pause_status(id).unwrap().resumable,
        "an input refused before processing loses continuity"
    );
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id].in_seq_no,
        before
    );
    world.lock().unwrap().send_gate = None;
    gate.release.notify_waiters();
    assert!(soon(held).await.unwrap().is_err());
    soon(other).await.unwrap().unwrap();
    assert!(a.resume_chat(id).await.is_err());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn protected_message_without_consumer_suspends_and_keeps_keys() {
    protected_persistence_refusal_does_not_suspend_or_write().await;
    protected_file_is_received_in_ram_without_a_saved_copy().await;
    protected_uncertain_send_refuses_instead_of_promising_durable_retry().await;
    let (a, b, store, world, id) = protected_pair(2.0, 2).await;
    b.send_message(id, "no consumer", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Suspended);
    let state = store.state.lock().unwrap();
    let chat = &state.as_ref().unwrap().chats[&id];
    assert!(chat.key.is_some());
    assert!(chat.outbox.is_empty() && chat.deliveries.is_empty());
    assert!(world.lock().unwrap().discards.is_empty());
}
#[tokio::test]
async fn protected_async_error_suspends_without_key_or_error_logging() {
    let (a, b, _store, world, id) = protected_pair(2.0, 2).await;
    assert!(a.on("MessageReceived", "inline", |_| Ok(())).is_err());
    a.on_async("MessageReceived", "failure", |_| async {
        Err(Error::MediaKind {
            kind: "private details".into(),
        })
    })
    .unwrap();
    b.send_message(id, "private plaintext", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    a.settle().await.unwrap();
    assert_eq!(a.status(id).unwrap().state, ChatState::Suspended);
    assert!(a.read_history(id, 10).unwrap().is_empty());
    assert!(a.status(id).unwrap().key_fingerprint.is_some());
    assert!(world.lock().unwrap().discards.is_empty());
}
async fn protected_uncertain_send_refuses_instead_of_promising_durable_retry() {
    let (a, _b, store, world, id) = protected_pair(2.0, 2).await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    assert!(matches!(
        a.send_message(id, "uncertain", None, None).await,
        Err(Error::Paused)
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Suspended);
    assert!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .key
            .is_some()
    );
    assert!(world.lock().unwrap().discards.is_empty());
}
async fn protected_file_is_received_in_ram_without_a_saved_copy() {
    let (a, b, _store, world, id) = protected_pair(2.0, 3).await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    a.on_async("MessageReceived", "media", move |e| {
        let sink = Arc::clone(&sink);
        async move {
            if let Event::MessageReceived(m) = e {
                sink.lock().unwrap().push(m);
            }
            Ok(())
        }
    })
    .unwrap();
    let content = b"bounded RAM file".repeat(100);
    b.send_file(
        id,
        FileSource::bytes(content.clone(), "file.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    a.settle().await.unwrap();
    let message = got.lock().unwrap()[0].clone();
    assert_eq!(a.receive_message_file(&message).await.unwrap(), content);
    assert!(a.auto_save_secret_chats().unwrap().is_none());
    assert!(a.read_saved_messages(id).is_err());
}
async fn protected_persistence_refusal_does_not_suspend_or_write() {
    let (a, _b, _store, _world, id) = protected_pair(2.0, 2).await;
    let directory = tempfile::tempdir().unwrap();
    assert!(
        a.start_auto_save_secret_chats(directory.path())
            .await
            .is_err()
    );
    assert!(a.delete_secret_chat(id).await.is_err());
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    assert!(!a.status(id).unwrap().paused);
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
}
