#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    error::Error,
    manager::{Event, MessageReceived},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use support::*;
fn received(manager: &Manager) -> Arc<Mutex<Vec<MessageReceived>>> {
    let records = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&records);
    manager
        .on("MessageReceived", "collect", move |event| {
            if let Event::MessageReceived(message) = event {
                sink.lock().unwrap().push(*message);
            }
            Ok(())
        })
        .unwrap();
    records
}
#[tokio::test]
async fn creating_leaves_chat_awaiting_acceptance() {
    let (a, _b, _) = managers();
    let chat = a.create(peer(2000)).await.unwrap();
    assert_eq!(chat.state, ChatState::Requested);
    assert!(chat.key_fingerprint.is_none());
}
#[tokio::test]
async fn both_ends_report_same_fingerprint() {
    let (a, b, _, id) = established().await;
    let aa = a.status(id).unwrap();
    let bb = b.status(id).unwrap();
    assert_eq!(aa.key_fingerprint, bb.key_fingerprint);
    assert_eq!(aa.key_hash, bb.key_hash);
    assert_eq!(aa.state, ChatState::Ready);
    assert_eq!(bb.state, ChatState::Ready);
}
#[tokio::test]
async fn sides_disagree_about_originator() {
    let (a, b, _, id) = established().await;
    assert!(a.status(id).unwrap().is_outbound);
    assert!(!b.status(id).unwrap().is_outbound);
}
#[tokio::test]
async fn incoming_request_is_reported() {
    let (a, b, world) = managers();
    let seen = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&seen);
    b.on("ChatRequested", "requested", move |event| {
        if let Event::ChatRequested { peer_user_id, .. } = event {
            assert_eq!(peer_user_id, 1000);
            sink.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    })
    .unwrap();
    a.create(peer(2000)).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn both_ends_emit_ready() {
    let (a, b, world) = managers();
    let ready = Arc::new(AtomicUsize::new(0));
    for manager in [&a, &b] {
        let sink = Arc::clone(&ready);
        manager
            .on("ChatReady", "ready", move |_| {
                sink.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
    }
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(ready.load(Ordering::SeqCst), 2);
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
}
#[tokio::test]
async fn message_crosses_as_text() {
    saved_messages_survive_protocol_delete().await;
    let (a, b, world, id) = established().await;
    let got = received(&b);
    a.send_message(id, "hello there", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(got.lock().unwrap()[0].text, "hello there");
}
#[tokio::test]
async fn messages_arrive_in_order() {
    service_and_message_archive_keeps_sequence_order().await;
    let (a, b, world, id) = established().await;
    let got = received(&b);
    for i in 0..5 {
        a.send_message(id, &format!("message {i}"), Some(vec![]), None)
            .await
            .unwrap();
    }
    pump(&a, &b, &world).await;
    assert_eq!(
        got.lock()
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
        (0..5).map(|i| format!("message {i}")).collect::<Vec<_>>()
    );
}
#[tokio::test]
async fn both_directions_work() {
    let (a, b, world, id) = established().await;
    let to_b = received(&b);
    let to_a = received(&a);
    a.send_message(id, "originator", Some(vec![]), None)
        .await
        .unwrap();
    b.send_message(id, "acceptor", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(to_b.lock().unwrap()[0].text, "originator");
    assert_eq!(to_a.lock().unwrap()[0].text, "acceptor");
}
#[tokio::test]
async fn history_returns_recent_messages_in_order() {
    let (a, b, world, id) = established().await;
    for i in 0..4 {
        a.send_message(id, &format!("m{i}"), Some(vec![]), None)
            .await
            .unwrap();
    }
    pump(&a, &b, &world).await;
    assert_eq!(
        b.read_history(id, 10)
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
        vec!["m0", "m1", "m2", "m3"]
    );
    assert_eq!(
        b.read_history(id, 2)
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
        vec!["m2", "m3"]
    );
    assert!(b.read_history(id, 0).unwrap().is_empty());
}
#[tokio::test]
async fn send_returns_original_random_id() {
    uncertain_send_exposes_identity_and_reuses_ciphertext().await;
    let (a, b, world, id) = established().await;
    let got = received(&b);
    let sent = a.send_message(id, "x", Some(vec![]), None).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(sent, got.lock().unwrap()[0].random_id);
}
#[tokio::test]
async fn sending_before_ready_is_refused() {
    let (a, _, _) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    assert!(matches!(
        a.send_message(id, "early", None, None).await,
        Err(Error::ChatNotReady { .. })
    ));
}
#[tokio::test]
async fn sending_closed_chat_is_refused() {
    let (a, _, _, id) = established().await;
    a.close(id).await.unwrap();
    assert!(matches!(
        a.send_message(id, "late", None, None).await,
        Err(Error::ChatClosed { .. })
    ));
}
#[tokio::test]
async fn closing_twice_is_idempotent() {
    let (a, _, world, id) = established().await;
    a.close(id).await.unwrap();
    a.close(id).await.unwrap();
    assert_eq!(world.lock().unwrap().discards, vec![(id, false)]);
}
#[tokio::test]
async fn unknown_status_and_history_are_refused() {
    let (a, _, _) = managers();
    assert!(matches!(a.status(999), Err(Error::UnknownChat { .. })));
    assert!(matches!(
        a.read_history(999, 1),
        Err(Error::UnknownChat { .. })
    ));
}
#[tokio::test]
async fn malformed_update_reports_failure_without_throwing() {
    let (a, _, _) = managers();
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    a.on("DecryptFailed", "failed", move |event| {
        assert_eq!(event.chat_id(), 424242);
        assert!(!format!("{event:?}").contains("cipher"));
        sink.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    use grammers_tl_types as tl;
    let update = tl::enums::Update::from(tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessageService {
            random_id: 1,
            chat_id: 424242,
            date: 1,
            bytes: vec![0; 64],
        }
        .into(),
        qts: 1,
    });
    a.handle_update(&update).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(a.list().unwrap().is_empty());
}
#[tokio::test]
async fn notify_layer_is_sent_immediately_after_ready() {
    let (a, b, world, id) = established().await;
    assert!(a.status(id).unwrap().capability_layer >= 73);
    assert!(b.status(id).unwrap().capability_layer >= 73);
    let frames = world.lock().unwrap().sent.clone();
    assert_eq!(frames.len(), 2);
    for frame in frames {
        let wrapper = kiaro_secret_chat::framing::unwrap(&frame.outgoing.body).unwrap();
        assert!(
            matches!(wrapper.message,kiaro_secret_chat::enums::DecryptedMessage::Service(s) if matches!(&s.action,kiaro_secret_chat::enums::DecryptedMessageAction::NotifyLayer(n) if n.layer>=73))
        );
    }
}
#[tokio::test]
async fn formatted_text_is_parsed() {
    let (a, b, world, id) = established().await;
    let got = received(&b);
    a.send_message(id, "**hello**", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    let got = got.lock().unwrap();
    assert_eq!(got[0].text, "hello");
    assert!(got[0].entities.as_ref().is_some_and(|e| !e.is_empty()));
}
#[tokio::test]
async fn peer_discard_closes_locally_once() {
    let (a, b, world, id) = established().await;
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    b.on("ChatClosed", "closed", move |_| {
        sink.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    a.close(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn sync_handler_runs_inline_and_errors_are_isolated() {
    let (a, _, _) = managers();
    let count = Arc::new(AtomicUsize::new(0));
    a.on("ChatRequested", "bad", |_| Err(Error::Storage))
        .unwrap();
    let sink = Arc::clone(&count);
    a.on("ChatRequested", "good", move |_| {
        sink.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    let request = grammers_tl_types::types::EncryptedChatRequested {
        folder_id: None,
        id: 99,
        access_hash: 88,
        date: 1,
        admin_id: 2000,
        participant_id: 1000,
        g_a: {
            let dh = kiaro_secret_chat::dh::DhConfig::new(2, &prime()).unwrap();
            dh.public_value(&kiaro_secret_chat::dh::SecretExponent::generate().unwrap())
        },
    };
    a.handle_encryption(request.into()).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn async_handler_is_actually_run() {
    handler_can_stop_without_cancelling_itself().await;
    let (a, b, world) = managers();
    let reached = Arc::new(tokio::sync::Notify::new());
    let sink = Arc::clone(&reached);
    b.on_async("ChatRequested", "async_handler", move |_| {
        let sink = Arc::clone(&sink);
        async move {
            sink.notify_one();
            Ok(())
        }
    })
    .unwrap();
    a.create(peer(2000)).await.unwrap();
    pump(&a, &b, &world).await;
    tokio::time::timeout(std::time::Duration::from_secs(2), reached.notified())
        .await
        .unwrap();
}
async fn handler_can_stop_without_cancelling_itself() {
    let (a, b, world) = managers();
    b.start().await.unwrap();
    let done = Arc::new(tokio::sync::Notify::new());
    let manager = b.clone();
    let sink = Arc::clone(&done);
    b.on_async("ChatRequested", "stopping_handler", move |_| {
        let manager = manager.clone();
        let sink = Arc::clone(&sink);
        async move {
            manager.stop().await?;
            sink.notify_one();
            Ok(())
        }
    })
    .unwrap();
    a.create(peer(2000)).await.unwrap();
    pump(&a, &b, &world).await;
    tokio::time::timeout(std::time::Duration::from_secs(2), done.notified())
        .await
        .unwrap();
}
#[tokio::test]
async fn start_and_stop_are_idempotent() {
    explicit_off_before_first_start_stays_off().await;
    auto_save_off_preserves_existing_copies().await;
    let (a, _, _) = managers();
    a.start().await.unwrap();
    a.start().await.unwrap();
    a.stop().await.unwrap();
    a.stop().await.unwrap();
    a.start().await.unwrap();
    a.stop().await.unwrap();
}
async fn uncertain_send_exposes_identity_and_reuses_ciphertext() {
    let (a, b, world, id) = established().await;
    world.lock().unwrap().fail_send = Some(kiaro_secret_chat::manager::RpcFailure::Transient);
    let error = a
        .send_message(id, "uncertain", Some(vec![]), None)
        .await
        .unwrap_err();
    let Error::SendPending { random_id, .. } = error else {
        panic!("Expected retained identity")
    };
    let original = world.lock().unwrap().sent.last().unwrap().clone();
    a.retry_pending(id).await.unwrap();
    let repeated = world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(original.outgoing.frame, repeated.outgoing.frame);
    assert_eq!(repeated.outgoing.random_id, random_id);
    pump(&a, &b, &world).await;
    assert_eq!(b.read_history(id, 10).unwrap().len(), 1);
}
#[tokio::test]
async fn accepting_retries_original_exponent() {
    let (a, b, world) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    world.lock().unwrap().fail_accept = Some(kiaro_secret_chat::manager::RpcFailure::Transient);
    assert!(b.accept(id).await.is_err());
    world.lock().unwrap().fail_accept = None;
    b.accept(id).await.unwrap();
    let calls = &world.lock().unwrap().accepts;
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
}
async fn saved_messages_survive_protocol_delete() {
    let (a, b, world, id) = established().await;
    let directory = tempfile::tempdir().unwrap();
    b.start_auto_save_secret_chats(directory.path())
        .await
        .unwrap();
    a.send_message(id, "keep", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    a.delete_secret_chat(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.read_saved_messages(id).unwrap()[0]["text"], "keep");
}
async fn auto_save_off_preserves_existing_copies() {
    let (a, b, world, id) = established().await;
    let directory = tempfile::tempdir().unwrap();
    b.start_auto_save_secret_chats(directory.path())
        .await
        .unwrap();
    a.send_message(id, "before", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    b.stop_auto_save_secret_chats().await.unwrap();
    a.send_message(id, "after", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.read_saved_messages(id).unwrap().len(), 1);
}
async fn explicit_off_before_first_start_stays_off() {
    let (a, _, _) = managers();
    a.stop_auto_save_secret_chats().await.unwrap();
    a.start().await.unwrap();
    assert!(a.auto_save_secret_chats().unwrap().is_none());
}
async fn service_and_message_archive_keeps_sequence_order() {
    let (a, b, world, id) = established().await;
    let directory = tempfile::tempdir().unwrap();
    b.start_auto_save_secret_chats(directory.path())
        .await
        .unwrap();
    a.send_message(id, "before", Some(vec![]), None)
        .await
        .unwrap();
    a.set_ttl(id, 5).await.unwrap();
    a.send_message(id, "after", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let records = b.read_saved_messages(id).unwrap();
    assert_eq!(records[0]["text"], "before");
    assert_eq!(records[1]["type"], "service");
    assert_eq!(records[2]["text"], "after");
    assert_eq!(records[2]["ttl"], 5);
}
#[tokio::test]
async fn corrupt_frame_is_event_and_does_not_escape_update_loop() {
    let (a, b, world, id) = established().await;
    let got = received(&b);
    let failures = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&failures);
    b.on("DecryptFailed", "corrupt", move |_| {
        sink.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    a.send_message(id, "this one is fine", None, None)
        .await
        .unwrap();
    let mut corrupted = world
        .lock()
        .unwrap()
        .updates
        .get(&2000)
        .unwrap()
        .back()
        .unwrap()
        .clone();
    if let grammers_tl_types::enums::Update::NewEncryptedMessage(u) = &mut corrupted {
        if let grammers_tl_types::enums::EncryptedMessage::Message(m) = &mut u.message {
            m.bytes[32] ^= 1;
        } else {
            panic!("Expected content frame");
        }
    } else {
        panic!("Expected encrypted update");
    }
    pump(&a, &b, &world).await;
    b.handle_update(&corrupted).await;
    assert_eq!(failures.load(Ordering::SeqCst), 1);
    assert_eq!(got.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn decrypt_failure_event_contains_no_plaintext_or_ciphertext() {
    let (a, b, world, id) = established().await;
    let private_text = "private-message-body";
    a.send_message(id, private_text, None, None).await.unwrap();
    let mut corrupted = world
        .lock()
        .unwrap()
        .updates
        .get(&2000)
        .unwrap()
        .back()
        .unwrap()
        .clone();
    let ciphertext =
        if let grammers_tl_types::enums::Update::NewEncryptedMessage(u) = &mut corrupted {
            if let grammers_tl_types::enums::EncryptedMessage::Message(m) = &mut u.message {
                m.bytes[32] ^= 1;
                hex::encode(&m.bytes)
            } else {
                panic!("Expected content frame");
            }
        } else {
            panic!("Expected encrypted update");
        };
    let rendered = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&rendered);
    b.on("DecryptFailed", "redaction", move |e| {
        let reason = match &e {
            Event::DecryptFailed { reason, .. } => *reason,
            _ => panic!("Expected failure"),
        };
        sink.lock().unwrap().push(format!("{e:?} {reason}"));
        Ok(())
    })
    .unwrap();
    b.handle_update(&corrupted).await;
    let values = rendered.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert!(!values[0].contains(private_text));
    assert!(!values[0].contains(&ciphertext));
    assert!(!values[0].contains(&ciphertext[..32]));
}
#[tokio::test]
async fn text_reply_reaches_peer_with_original_reply_id() {
    let (a, b, world, id) = established().await;
    let got = received(&b);
    a.send_message(id, "reply", None, Some(987654321))
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(got.lock().unwrap()[0].reply_to, Some(987654321));
}
#[tokio::test]
async fn ordinary_message_has_no_reply_id() {
    let (a, b, world, id) = established().await;
    let got = received(&b);
    a.send_message(id, "ordinary", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    assert!(got.lock().unwrap()[0].reply_to.is_none());
}
#[tokio::test]
async fn runtime_history_keeps_only_configured_recent_messages() {
    use kiaro_secret_chat::{
        manager::{Options, SecretChatManager},
        storage::MemoryStorage,
    };
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = SecretChatManager::new(
        aa,
        MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options {
            history_limit: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    for n in 0..5 {
        a.send_message(id, &format!("m{n}"), None, None)
            .await
            .unwrap();
    }
    pump(&a, &b, &world).await;
    assert_eq!(
        b.read_history(id, 100)
            .unwrap()
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["m2", "m3", "m4"]
    );
}
