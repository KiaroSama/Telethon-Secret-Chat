#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
#[path = "support_manager/transient.rs"]
mod transient;
use kiaro_secret_chat::{
    chat::ChatState,
    custody::{ProtectedFileStorage, TransientLimits, TransientStorage},
    error::Result,
    manager::{Options, SecretChatManager},
    storage::Storage,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::*;
use transient::*;
async fn soon<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
#[tokio::test]
async fn disabling_requests_preserves_existing_conversation() {
    let (a, b, w, id) = pair(TestStorage::default(), TransientLimits::default()).await;
    let incoming = b.create(peer(1000)).await.unwrap().id;
    pump(&a, &b, &w).await;
    a.accept(incoming).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(a.status(incoming).unwrap().state, ChatState::Ready);
    a.set_accepting_requests(false).unwrap();
    let before = w.lock().unwrap().request_ids.len();
    assert!(a.create(peer(2000)).await.is_err());
    assert_eq!(w.lock().unwrap().request_ids.len(), before);
    let refused = b.create(peer(1000)).await.unwrap().id;
    pump(&a, &b, &w).await;
    assert!(a.status(refused).is_err());
    a.send_message(id, "still live", None, None).await.unwrap();
    assert!(!a.is_suspended(id).unwrap());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn protected_shutdown_cancels_callback_and_preserves_keys() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), TransientLimits::default()).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    let cleaned = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&cleaned);
    a.on_async_with_cleanup(
        "MessageReceived",
        "waiting",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let done = Arc::clone(&done);
            async move {
                done.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(id, "waiting", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    soon(entered.notified()).await;
    a.stop().await.unwrap();
    assert!(cleaned.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
    assert!(a.is_suspended(id).unwrap());
    assert!(store.load().unwrap().unwrap().chats[&id].key.is_some());
    b.stop().await.unwrap();
}
#[tokio::test]
async fn cancelled_protected_callback_can_stop_without_join_cycle() {
    let (a, b, w, id) = pair(TestStorage::default(), TransientLimits::default()).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    let done = Arc::new(AtomicBool::new(false));
    let completed = Arc::clone(&done);
    let manager = a.clone();
    a.on_async_with_cleanup(
        "MessageReceived",
        "waiting",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let manager = manager.clone();
            let completed = Arc::clone(&completed);
            async move {
                manager.stop().await?;
                completed.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(id, "cleanup", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    soon(entered.notified()).await;
    soon(a.stop()).await.unwrap();
    assert!(done.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
    b.stop().await.unwrap();
}
#[tokio::test]
async fn failed_suspension_commit_still_cancels_owned_callback() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), TransientLimits::default()).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    let done = Arc::new(AtomicBool::new(false));
    let completed = Arc::clone(&done);
    a.on_async_with_cleanup(
        "MessageReceived",
        "waiting",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let completed = Arc::clone(&completed);
            async move {
                completed.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(id, "active", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    soon(entered.notified()).await;
    store.failures.store(1, Ordering::Release);
    assert!(a.close(id).await.is_err());
    a.settle().await.unwrap();
    assert!(done.load(Ordering::Acquire));
    assert!(a.is_suspended(id).unwrap());
    assert_eq!(a.active_handler_tasks(), 0);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn reconstructed_protected_manager_refuses_without_erasing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.json");
    let (a, b, w, id) = pair(
        ProtectedFileStorage::open(&path).unwrap(),
        TransientLimits::default(),
    )
    .await;
    a.send_message(id, "crash window", None, None)
        .await
        .unwrap();
    let before = ProtectedFileStorage::open(&path).err();
    assert!(before.is_some());
    let key = a.status(id).unwrap().key_fingerprint;
    a.stop().await.unwrap();
    let persisted = ProtectedFileStorage::open(&path).unwrap();
    let before = persisted.load().unwrap().unwrap().chats[&id].clone();
    drop(persisted);
    let count = w.lock().unwrap().sent.len();
    let restored = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        TransientStorage::new(
            ProtectedFileStorage::open(&path).unwrap(),
            "11".repeat(32),
            TransientLimits::default(),
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    restored.start().await.unwrap();
    assert!(restored.is_suspended(id).unwrap());
    assert!(
        restored
            .send_message(id, "refuse", None, None)
            .await
            .is_err()
    );
    assert_eq!(w.lock().unwrap().sent.len(), count);
    assert_eq!(restored.status(id).unwrap().key_fingerprint, key);
    assert_eq!(restored.status(id).unwrap().state, ChatState::Suspended);
    restored.stop().await.unwrap();
    let after = ProtectedFileStorage::open(&path)
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .chats[&id]
        .clone();
    assert_eq!(before.out_seq_no, after.out_seq_no);
    assert_eq!(before.in_seq_no, after.in_seq_no);
    assert_eq!(
        before.key.as_ref().unwrap().fingerprint(),
        after.key.as_ref().unwrap().fingerprint()
    );
    assert!(w.lock().unwrap().discards.is_empty());
    b.stop().await.unwrap();
}
#[tokio::test]
async fn malformed_frame_suspends_without_erasing_key_or_advancing_counter() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), TransientLimits::default()).await;
    let before = store.load().unwrap().unwrap().chats[&id].clone();
    assert!(
        a.receive(id, b"bad synthetic frame", None, 123)
            .await
            .is_err()
    );
    assert!(a.is_suspended(id).unwrap());
    let after = store.load().unwrap().unwrap().chats[&id].clone();
    assert_eq!(
        before.key.as_ref().unwrap().fingerprint(),
        after.key.as_ref().unwrap().fingerprint()
    );
    assert_eq!(before.in_seq_no, after.in_seq_no);
    assert!(w.lock().unwrap().discards.is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn protected_live_rekey_keeps_counters_and_remains_serviceable() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), TransientLimits::default()).await;
    let before = store.load().unwrap().unwrap().chats[&id].clone();
    a.rekey(id).await.unwrap();
    pump(&a, &b, &w).await;
    let after = store.load().unwrap().unwrap().chats[&id].clone();
    assert_ne!(
        before.key.as_ref().unwrap().fingerprint(),
        after.key.as_ref().unwrap().fingerprint()
    );
    assert!(after.out_seq_no > before.out_seq_no);
    assert!(after.in_seq_no >= before.in_seq_no);
    assert!(after.exchange.is_none());
    assert!(after.outbox.is_empty());
    assert!(!a.is_suspended(id).unwrap());
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn protected_simultaneous_rekeys_converge_without_suspension() {
    let (a, b, w, id) = pair(TestStorage::default(), TransientLimits::default()).await;
    a.rekey(id).await.unwrap();
    b.rekey(id).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
    assert!(!a.status(id).unwrap().exchange_in_progress);
    assert!(!b.status(id).unwrap().exchange_in_progress);
    assert!(!a.is_suspended(id).unwrap());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn earlier_suspending_handler_prevents_later_delivery() {
    use kiaro_secret_chat::manager::Event;
    let (a, b, w, id) = pair(TestStorage::default(), TransientLimits::default()).await;
    let first = a.clone();
    a.on_async("MessageReceived", "suspend", move |event| {
        let first = first.clone();
        async move { first.close(event.chat_id()).await }
    })
    .unwrap();
    let deliveries = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&deliveries);
    a.on_async("MessageReceived", "later", move |_| {
        let seen = Arc::clone(&seen);
        async move {
            seen.store(true, Ordering::Release);
            Ok(())
        }
    })
    .unwrap();
    let diagnostics = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = Arc::clone(&diagnostics);
    a.on_async("DecryptFailed", "diagnostic", move |event| {
        let seen = Arc::clone(&seen);
        async move {
            if matches!(event, Event::DecryptFailed { .. }) {
                seen.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        }
    })
    .unwrap();
    b.send_message(id, "stop before later", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    a.settle().await.unwrap();
    a.settle().await.unwrap();
    assert!(!deliveries.load(Ordering::Acquire));
    assert_eq!(diagnostics.load(Ordering::Acquire), 1);
    assert!(a.is_suspended(id).unwrap());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn protected_delivery_requires_explicit_read_receipt() {
    use kiaro_secret_chat::{
        enums::{DecryptedMessage, DecryptedMessageAction},
        manager::Event,
    };
    let (a, b, w, id) = pair(TestStorage::default(), TransientLimits::default()).await;
    let got = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    a.on_async("MessageReceived", "collect", move |event| {
        let seen = Arc::clone(&seen);
        async move {
            if let Event::MessageReceived(message) = event {
                seen.lock().unwrap().push(message);
            }
            Ok(())
        }
    })
    .unwrap();
    b.set_ttl(id, 30).await.unwrap();
    b.send_message(id, "timed", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    a.settle().await.unwrap();
    assert_eq!(got.lock().unwrap()[0].ttl, 30);
    let has_read = |world: &World| {
        world.sent.iter().any(|effect|matches!(kiaro_secret_chat::framing::unwrap(&effect.outgoing.body).unwrap().message,DecryptedMessage::Service(s)if matches!(s.action,DecryptedMessageAction::ReadMessages(_))))
    };
    assert!(!has_read(&w.lock().unwrap()));
    let identity = got.lock().unwrap()[0].random_id;
    a.mark_read(id, vec![identity]).await.unwrap();
    assert!(has_read(&w.lock().unwrap()));
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn transient_file_preflight_does_not_expose_filename() {
    let (transport, _) = Fake::pair();
    let w = Arc::clone(&transport.world);
    let a = SecretChatManager::new(
        transport,
        TransientStorage::new(
            TestStorage::default(),
            "11".repeat(32),
            TransientLimits::default(),
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sentinel = "synthetic-confidential-filename.bin";
    let error = a
        .send_file(
            7,
            kiaro_secret_chat::media::FileSource::path(dir.path().join(sentinel)),
            kiaro_secret_chat::manager::SendFileOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(sentinel));
    assert!(!format!("{error:?}").contains(sentinel));
    assert!(w.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[test]
fn transient_limits_reject_boolean_task_count() {
    let mut value = serde_json::to_value(TransientLimits::default()).unwrap();
    value["tasks"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TransientLimits>(value).is_err());
    TransientLimits::default().validate().unwrap();
}
#[test]
fn transient_limits_reject_infinite_operation_deadline() {
    let limits = TransientLimits {
        operation_seconds: f64::INFINITY,
        ..Default::default()
    };
    assert!(limits.validate().is_err());
}
#[test]
fn transient_limits_reject_zero_attempts() {
    let limits = TransientLimits {
        attempts: 0,
        ..Default::default()
    };
    assert!(limits.validate().is_err());
}
#[tokio::test]
async fn task_budget_rejects_second_create_without_another_rpc() {
    let (transport, _) = Fake::pair();
    let w = Arc::clone(&transport.world);
    let gate = Arc::new(Gate::default());
    w.lock().unwrap().dh_gate = Some(Arc::clone(&gate));
    let limits = TransientLimits {
        tasks: 1,
        ..Default::default()
    };
    let a = SecretChatManager::new(
        transport,
        TransientStorage::new(TestStorage::default(), "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    let manager = a.clone();
    let task = tokio::spawn(async move { manager.create(peer(2000)).await });
    soon(gate.entered.notified()).await;
    assert!(a.create(peer(2000)).await.is_err());
    assert!(w.lock().unwrap().request_ids.is_empty());
    w.lock().unwrap().dh_gate = None;
    gate.release.notify_one();
    soon(task).await.unwrap().unwrap();
    assert_eq!(w.lock().unwrap().request_ids.len(), 1);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn changed_authorization_refuses_without_recovery_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.json");
    let (a, b, w, id) = pair(
        ProtectedFileStorage::open(&path).unwrap(),
        TransientLimits::default(),
    )
    .await;
    a.stop().await.unwrap();
    let before = std::fs::read(&path).unwrap();
    let count = w.lock().unwrap().sent.len();
    w.lock().unwrap().authorization = Some("22".repeat(32));
    let restored = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        TransientStorage::new(
            ProtectedFileStorage::open(&path).unwrap(),
            "11".repeat(32),
            TransientLimits::default(),
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    assert!(restored.start().await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(w.lock().unwrap().sent.len(), count);
    assert!(restored.status(id).unwrap().key_fingerprint.is_some());
    restored.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn real_grammers_authorization_surface_uses_caller_session_without_connection() {
    use grammers_session::{Session, SessionData, storages::MemorySession};
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    let key = std::array::from_fn(|i| i as u8);
    data.dc_options.get_mut(&2).unwrap().auth_key = Some(key);
    let session = Arc::new(MemorySession::from(data));
    let pool = grammers_mtsender::SenderPool::new(Arc::clone(&session), 12345);
    let _client = grammers_client::Client::new(pool.handle.clone());
    assert_eq!(session.home_dc_id().unwrap(), 2);
    assert_eq!(pool.handle.session.home_dc_id().unwrap(), 2);
    assert_eq!(
        pool.handle.session.dc_option(2).unwrap().unwrap().auth_key,
        Some(key)
    );
    assert!(Arc::strong_count(&session) >= 2); /* The original caller's runner is never spawned and no invocation occurs. */
}
#[tokio::test]
async fn gap_completion_and_duplicate_preserve_original_delivery_ids() {
    use kiaro_secret_chat::manager::Event;
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), TransientLimits::default()).await;
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&received);
    a.on_async("MessageReceived", "collect", move |event| {
        let seen = Arc::clone(&seen);
        async move {
            if let Event::MessageReceived(m) = event {
                seen.lock().unwrap().push((m.random_id, m.text));
            }
            Ok(())
        }
    })
    .unwrap();
    let first_id = b.send_message(id, "first", None, None).await.unwrap();
    let second_id = b.send_message(id, "second", None, None).await.unwrap();
    let (first, second) = {
        let mut world = w.lock().unwrap();
        let updates = world.updates.get_mut(&1000).unwrap();
        (updates.pop_front().unwrap(), updates.pop_front().unwrap())
    };
    a.handle_update(&second).await;
    a.settle().await.unwrap();
    assert!(received.lock().unwrap().is_empty());
    a.handle_update(&first).await;
    a.settle().await.unwrap();
    assert_eq!(
        *received.lock().unwrap(),
        [(first_id, "first".into()), (second_id, "second".into())]
    );
    let counter = store.load().unwrap().unwrap().chats[&id].in_seq_no;
    a.handle_update(&second).await;
    a.settle().await.unwrap();
    assert_eq!(received.lock().unwrap().len(), 2);
    assert_eq!(store.load().unwrap().unwrap().chats[&id].in_seq_no, counter);
    assert!(!a.is_suspended(id).unwrap());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn gap_payload_stays_volatile_until_retention_expiry_suspends() {
    let store = TestStorage::default();
    let limits = TransientLimits {
        retention_seconds: 0.5,
        ..Default::default()
    };
    let (a, b, w, id) = pair(store.clone(), limits).await;
    b.send_message(id, "first synthetic gap", None, None)
        .await
        .unwrap();
    b.send_message(id, "second synthetic gap", None, None)
        .await
        .unwrap();
    let second = {
        let mut world = w.lock().unwrap();
        let updates = world.updates.get_mut(&1000).unwrap();
        let _ = updates.pop_front().unwrap();
        updates.pop_front().unwrap()
    };
    a.handle_update(&second).await;
    let before = store.load().unwrap().unwrap().chats[&id].clone();
    assert!(before.gaps.is_empty());
    assert!(
        !serde_json::to_string(&store.load().unwrap())
            .unwrap()
            .contains("second synthetic gap")
    );
    soon(async {
        while !a.is_suspended(id).unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        a.is_suspended(id).unwrap(),
        "the owned expiry timer acts without another public operation"
    );
    assert!(
        a.send_message(id, "refuse after expiry", None, None)
            .await
            .is_err()
    );
    assert!(a.is_suspended(id).unwrap());
    let after = store.load().unwrap().unwrap().chats[&id].clone();
    assert_eq!(before.in_seq_no, after.in_seq_no);
    assert_eq!(
        before.key.as_ref().unwrap().fingerprint(),
        after.key.as_ref().unwrap().fingerprint()
    );
    assert!(w.lock().unwrap().discards.is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn deadline_drops_transport_future_without_a_stale_owned_rpc() {
    let store = TestStorage::default();
    let limits = TransientLimits {
        operation_seconds: 0.04,
        ..Default::default()
    };
    let (a, b, w, id) = pair(store.clone(), limits).await;
    let gate = Arc::new(Gate::default());
    w.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    assert!(
        a.send_message(id, "uncertain synthetic send", None, None)
            .await
            .is_err()
    );
    assert_eq!(w.lock().unwrap().active_sends, 0);
    assert!(a.is_suspended(id).unwrap());
    assert_eq!(a.active_handler_tasks(), 0);
    gate.release.notify_one();
    tokio::task::yield_now().await;
    assert!(store.load().unwrap().unwrap().chats[&id].key.is_some());
    assert!(store.load().unwrap().unwrap().chats[&id].outbox.is_empty());
    assert!(w.lock().unwrap().discards.is_empty());
    assert!(b.read_history(id, 1).unwrap().is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
