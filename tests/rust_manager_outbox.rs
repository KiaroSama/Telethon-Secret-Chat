#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    enums,
    error::Error,
    framing,
    manager::{Event, Options, PermanentError, RpcFailure, SecretChatManager},
    protocol::Protocol,
    storage::MemoryStorage,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::*;
type StoredManager = SecretChatManager<TestStorage, Fake>;
async fn pair() -> (
    StoredManager,
    Manager,
    TestStorage,
    Fake,
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
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, store, transport, world, id)
}
async fn bounded<F: std::future::Future>(f: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), f)
        .await
        .unwrap()
}
#[tokio::test]
async fn reconstruction_retries_original_uncertain_wire_identity() {
    let (a, b, store, transport, world, id) = pair().await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let baseline = store.state.lock().unwrap().as_ref().unwrap().chats[&id].out_seq_no;
    assert!(matches!(
        a.send_message(id, "uncertain send", None, None).await,
        Err(Error::SendPending { .. })
    ));
    let original = world.lock().unwrap().sent.last().unwrap().clone();
    drop(a);
    let recovered = SecretChatManager::new(
        transport,
        store.clone(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    recovered.start().await.unwrap();
    let repeated = world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(repeated.outgoing.frame, original.outgoing.frame);
    assert_eq!(repeated.outgoing.random_id, original.outgoing.random_id);
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id].out_seq_no,
        baseline + 1
    );
    assert!(
        !store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .outbox
            .values()
            .any(|r| r.pending)
    );
    pump(&recovered, &b, &world).await;
    assert_eq!(b.read_history(id, 1).unwrap()[0].text, "uncertain send");
}
#[tokio::test]
async fn deleting_uncertain_content_never_retries_its_plaintext() {
    let (a, _b, store, _transport, world, id) = pair().await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending { random_id, .. } = a
        .send_message(id, "DELETE-ME-SYNTHETIC", None, None)
        .await
        .unwrap_err()
    else {
        panic!("Expected pending identity")
    };
    let initial = world.lock().unwrap().sent.len();
    a.delete_messages(id, vec![random_id]).await.unwrap();
    for effect in &world.lock().unwrap().sent[initial..] {
        let wrapper = framing::unwrap(&effect.outgoing.body).unwrap();
        assert!(
            matches!(wrapper.message,enums::DecryptedMessage::Service(s)if matches!(&s.action,enums::DecryptedMessageAction::DeleteMessages(_)))
        );
    }
    for row in store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
    {
        assert!(
            !row.body
                .windows(b"DELETE-ME-SYNTHETIC".len())
                .any(|w| w == b"DELETE-ME-SYNTHETIC")
        );
    }
}
#[tokio::test]
async fn flush_with_failed_rpc_withdraws_retained_content_and_history() {
    let (a, b, store, _transport, world, id) = pair().await;
    b.send_message(id, "history", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending { random_id, .. } = a
        .send_message(id, "FLUSH-ME-SYNTHETIC", None, None)
        .await
        .unwrap_err()
    else {
        panic!("Expected pending identity")
    };
    world
        .lock()
        .unwrap()
        .fail_sends
        .insert(random_id, RpcFailure::Transient);
    assert!(matches!(
        a.flush_history(id).await,
        Err(Error::SendPending { .. })
    ));
    assert!(a.read_history(id, 100).unwrap().is_empty());
    for row in store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
    {
        assert!(
            !row.body
                .windows(b"FLUSH-ME-SYNTHETIC".len())
                .any(|w| w == b"FLUSH-ME-SYNTHETIC")
        );
    }
}
#[tokio::test]
async fn cancelled_rpc_keeps_original_frame_retryable() {
    late_chat_ended_response_cannot_close_new_generation().await;
    let (a, b, store, _transport, world, id) = pair().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let call =
        tokio::spawn(async move { manager.send_message(id, "cancelled RPC", None, None).await });
    bounded(gate.entered.notified()).await;
    let original = world.lock().unwrap().sent.last().unwrap().clone();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    world.lock().unwrap().send_gate = None;
    a.retry_pending(id).await.unwrap();
    let repeated = world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(original.outgoing.frame, repeated.outgoing.frame);
    assert_eq!(original.outgoing.random_id, repeated.outgoing.random_id);
    assert!(
        !store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .outbox
            .values()
            .any(|r| r.pending)
    );
    assert_eq!(world.lock().unwrap().active_sends, 0);
    pump(&a, &b, &world).await;
}
#[tokio::test]
async fn concurrent_rpcs_follow_call_order_without_overlap() {
    use std::{future::Future, pin::Pin, task::Poll};
    let (a, b, _store, _transport, world, id) = pair().await;
    let initial = world.lock().unwrap().sent.len();
    let texts = (0..20).map(|n| n.to_string()).collect::<Vec<_>>();
    let mut calls = texts
        .iter()
        .map(|text| Box::pin(a.send_message(id, text, None, None)))
        .collect::<Vec<_>>();
    let mut results = (0..20).map(|_| None).collect::<Vec<_>>();
    std::future::poll_fn(|cx| {
        for (i, call) in calls.iter_mut().enumerate() {
            if results[i].is_none()
                && let Poll::Ready(value) = Pin::as_mut(call).poll(cx)
            {
                results[i] = Some(value);
            }
        }
        if results.iter().all(Option::is_some) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    let ids = results
        .into_iter()
        .map(|r| r.unwrap().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 20);
    let sent = world.lock().unwrap().sent[initial..].to_vec();
    assert_eq!(world.lock().unwrap().maximum_sends, 1);
    let first = sent[0].outgoing.seq_no;
    let mut received = Vec::new();
    for (index, effect) in sent.iter().enumerate() {
        assert_eq!(effect.outgoing.seq_no, first + 2 * index as i32);
        let wrapper = framing::unwrap(&effect.outgoing.body).unwrap();
        if let enums::DecryptedMessage::Message(m) = wrapper.message {
            received.push(m.message)
        } else {
            panic!("Expected ordinary message");
        }
    }
    assert_eq!(received, texts);
    pump(&a, &b, &world).await;
}
#[tokio::test]
async fn permanent_rejection_becomes_same_identity_self_delete() {
    let (a, b, store, _transport, world, id) = pair().await;
    let failed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&failed);
    a.on("SendFailed", "rejection", move |e| {
        if let Event::SendFailed {
            random_id, cause, ..
        } = e
        {
            sink.lock().unwrap().push((random_id, cause));
        }
        Ok(())
    })
    .unwrap();
    world.lock().unwrap().fail_send = Some(RpcFailure::Rejected(PermanentError::DataInvalid));
    let Error::SendPending { random_id, .. } = a
        .send_message(id, "never deliverable", None, None)
        .await
        .unwrap_err()
    else {
        panic!("Expected pending identity")
    };
    assert_eq!(*failed.lock().unwrap(), [(random_id, "DataInvalidError")]);
    let row = store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
        .find(|r| r.random_id == random_id)
        .unwrap()
        .clone();
    assert!(row.service);
    assert!(row.pending);
    assert!(
        matches!(framing::unwrap(&row.body).unwrap().message,enums::DecryptedMessage::Service(s)if matches!(s.action,enums::DecryptedMessageAction::DeleteMessages(_)))
    );
    a.send_message(id, "next", None, None).await.unwrap();
    assert!(
        !store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .outbox
            .values()
            .any(|r| r.pending)
    );
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    pump(&a, &b, &world).await;
}
#[tokio::test]
async fn second_permanent_protocol_rejection_closes_chat() {
    let (a, _b, _store, _transport, world, id) = pair().await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Permanent);
    assert!(matches!(
        a.set_ttl(id, 5).await,
        Err(Error::SendPending { .. })
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    world.lock().unwrap().fail_send = Some(RpcFailure::Permanent);
    assert!(matches!(
        a.retry_pending(id).await,
        Err(Error::ChatClosed { .. })
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
}
#[tokio::test]
async fn server_gone_closes_locally_without_discard_rpc() {
    let (a, _b, _store, _transport, world, id) = pair().await;
    let closed = Arc::new(Mutex::new(0));
    let sink = Arc::clone(&closed);
    a.on("ChatClosed", "closed", move |_| {
        *sink.lock().unwrap() += 1;
        Ok(())
    })
    .unwrap();
    world.lock().unwrap().fail_send = Some(RpcFailure::ChatEnded);
    assert!(matches!(
        a.send_message(id, "too late", None, None).await,
        Err(Error::ChatClosed { .. })
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(*closed.lock().unwrap(), 1);
    assert!(world.lock().unwrap().discards.is_empty());
}
#[tokio::test]
async fn stuck_record_does_not_block_later_protocol_retry() {
    let (a, _b, store, transport, world, id) = pair().await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending {
        random_id: stuck, ..
    } = a.send_message(id, "stuck", None, None).await.unwrap_err()
    else {
        panic!("Expected pending identity")
    };
    // Simulate a crash snapshot that already committed a later protocol action.
    drop(a);
    let mut protocol = Protocol::open(store.clone(), "11".repeat(32)).unwrap();
    protocol.start().unwrap();
    protocol
        .send_action(id, enums::DecryptedMessageAction::Noop)
        .unwrap();
    protocol.stop().unwrap();
    drop(protocol);
    let revived = SecretChatManager::new(
        transport,
        store.clone(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    world
        .lock()
        .unwrap()
        .fail_sends
        .insert(stuck, RpcFailure::Transient);
    assert!(
        matches!(revived.retry_pending(id).await,Err(Error::SendPending{random_id,..})if random_id==stuck)
    );
    let pending = store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
        .filter(|r| r.pending)
        .map(|r| r.random_id)
        .collect::<Vec<_>>();
    assert_eq!(pending, [stuck]);
}
async fn late_chat_ended_response_cannot_close_new_generation() {
    let (a, b, _store, _transport, world, id) = pair().await;
    let gate = Arc::new(Gate::default());
    {
        let mut w = world.lock().unwrap();
        w.send_gate = Some(Arc::clone(&gate));
        w.fail_send = Some(RpcFailure::ChatEnded);
    }
    let manager = a.clone();
    let old = tokio::spawn(async move { manager.send_message(id, "old RPC", None, None).await });
    bounded(gate.entered.notified()).await;
    a.stop().await.unwrap();
    world.lock().unwrap().send_gate = None;
    a.start().await.unwrap();
    gate.release.notify_one();
    assert!(matches!(
        bounded(old).await.unwrap(),
        Err(Error::SendPending { .. })
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    pump(&a, &b, &world).await;
}
#[tokio::test]
async fn request_arriving_during_start_recovery_is_not_lost() {
    let (a, b, store, transport, world, id) = pair().await;
    world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    assert!(a.send_message(id, "owed", None, None).await.is_err());
    drop(a);
    let revived =
        SecretChatManager::new(transport, store, "11".repeat(32), Options::default()).unwrap();
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = revived.clone();
    let starting = tokio::spawn(async move { manager.start().await });
    bounded(gate.entered.notified()).await;
    let incoming = b.create(peer(1000)).await.unwrap().id;
    pump(&revived, &b, &world).await;
    assert_eq!(revived.status(incoming).unwrap().state, ChatState::Waiting);
    world.lock().unwrap().send_gate = None;
    gate.release.notify_one();
    bounded(starting).await.unwrap().unwrap();
    assert!(revived.list().unwrap().iter().any(|c| c.id == incoming));
}
#[tokio::test]
async fn service_effect_storage_failure_preserves_mailbox_and_live_state() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (a, b, store, _transport, world, id) = pair().await;
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    a.on("ServiceActionReceived", "service", move |_| {
        sink.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    b.set_ttl(id, 9).await.unwrap();
    let envelope = world
        .lock()
        .unwrap()
        .updates
        .get_mut(&1000)
        .unwrap()
        .pop_front()
        .unwrap();
    let grammers_tl_types::enums::Update::NewEncryptedMessage(update) = envelope else {
        panic!("Expected encrypted update")
    };
    let grammers_tl_types::enums::EncryptedMessage::Service(message) = update.message else {
        panic!("Expected service frame")
    };
    store
        .fail_on_commit
        .store(store.commits.load(Ordering::Acquire) + 2, Ordering::Release);
    assert!(matches!(
        a.receive(id, &message.bytes, None, 1).await,
        Err(Error::Storage)
    ));
    assert_eq!(a.status(id).unwrap().ttl, 0);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    {
        let state = store.state.lock().unwrap();
        let chat = &state.as_ref().unwrap().chats[&id];
        assert_eq!(chat.ttl, 0);
        assert_eq!(chat.service_deliveries.len(), 1);
        assert!(
            chat.service_deliveries
                .values()
                .all(|d| d.applied.is_none())
        );
    }
    a.receive(id, &message.bytes, None, 1).await.unwrap();
    assert_eq!(a.status(id).unwrap().ttl, 9);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
