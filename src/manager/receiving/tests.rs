#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_support::seeded;
use crate::test_support::support;
use crate::{
    Serializable,
    chat::{ChatState, Envelope},
    crypto::{Direction, SharedKey, encrypt_frame},
};
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../tests/support_manager/mailbox_review_9c5.rs"]
mod review_9c5;
fn message(text: &str) -> crate::enums::DecryptedMessage {
    crate::types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1,
        ttl: 0,
        message: text.into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}
fn body(text: &str, raw: i32) -> Vec<u8> {
    crate::framing::wrap(message(text), 144, 1, raw * 2)
        .unwrap()
        .to_bytes()
}
fn update(id: i32, body: &[u8]) -> grammers_tl_types::enums::Update {
    let frame = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        body,
        Direction::Responder,
    )
    .unwrap();
    grammers_tl_types::types::UpdateNewEncryptedMessage {
        message: grammers_tl_types::types::EncryptedMessage {
            random_id: 999,
            chat_id: id,
            date: 0,
            bytes: frame,
            file: grammers_tl_types::enums::EncryptedFile::Empty,
        }
        .into(),
        qts: 1,
    }
    .into()
}
#[tokio::test]
async fn authenticated_unparseable_inner_closes_chat_once() {
    let (a, _store, w, id) = seeded::ready(Options::default()).await;
    let failed = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&failed);
    a.on("DecryptFailed", "count", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    let mut encoded = body("test", 0);
    let header = encoded.len() - message("test").to_bytes().len();
    encoded[header..header + 4].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());
    a.handle_update(&update(id, &encoded)).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(failed.load(Ordering::Acquire), 1);
    assert!(a.status(id).unwrap().key_fingerprint.is_none());
    assert_eq!(w.lock().unwrap().discards, vec![(id, false)]);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn unknown_authenticated_wrapper_header_is_nonfatal() {
    let (a, store, w, id) = seeded::ready(Options::default()).await;
    let failed = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&failed);
    a.on("DecryptFailed", "count", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    let before =
        serde_json::to_value(store.state.lock().unwrap().as_ref().unwrap().chats[&id].clone())
            .unwrap();
    let mut encoded = 0xdeadbeef_u32.to_le_bytes().to_vec();
    encoded.extend_from_slice(&[0; 60]);
    a.handle_update(&update(id, &encoded)).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    assert_eq!(failed.load(Ordering::Acquire), 1);
    assert!(w.lock().unwrap().discards.is_empty());
    assert_eq!(
        serde_json::to_value(store.state.lock().unwrap().as_ref().unwrap().chats[&id].clone())
            .unwrap(),
        before
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn malformed_runtime_mailbox_head_is_dropped_before_valid_delivery() {
    let (a, store, _w, id) = seeded::ready(Options::default()).await;
    let failed = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&failed);
    a.on("DecryptFailed", "count", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    a.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message.text);
        }
        Ok(())
    })
    .unwrap();
    a.protocol(|p| {
        p.test_replace_deliveries(
            id,
            [
                (
                    0,
                    Envelope {
                        body: 0xdeadbeef_u32.to_le_bytes().to_vec(),
                        file: None,
                        date: 1,
                    },
                ),
                (
                    1,
                    Envelope {
                        body: body("behind it", 1),
                        file: None,
                        date: 2,
                    },
                ),
            ]
            .into(),
        )
    })
    .unwrap();
    a.drain_deliveries(id).await.unwrap();
    assert_eq!(*got.lock().unwrap(), vec!["behind it"]);
    assert_eq!(failed.load(Ordering::Acquire), 1);
    assert!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .deliveries
            .is_empty()
    );
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn unsatisfiable_resend_closes_before_failure_event_with_reason() {
    let mut record = seeded::chat();
    record.is_outbound = false;
    let (a, _store, _world, id) = seeded::ready_chat(record, Options::default()).await;
    let order = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&order);
    a.on("ChatClosed", "closed", move |event| {
        if let Event::ChatClosed { reason, .. } = event {
            assert_eq!(reason, "a resend request could not be satisfied");
            seen.lock().unwrap().push("ChatClosed");
        }
        Ok(())
    })
    .unwrap();
    let seen = Arc::clone(&order);
    a.on("DecryptFailed", "failure", move |_| {
        seen.lock().unwrap().push("DecryptFailed");
        Ok(())
    })
    .unwrap();
    let service = crate::types::DecryptedMessageService {
        random_id: 1,
        action: crate::types::DecryptedMessageActionResend {
            start_seq_no: 200,
            end_seq_no: 200,
        }
        .into(),
    };
    let body = crate::framing::wrap(service.into(), 144, 0, 1)
        .unwrap()
        .to_bytes();
    let wire = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &body,
        Direction::Initiator,
    )
    .unwrap();
    let update = grammers_tl_types::types::UpdateNewEncryptedMessage {
        message: grammers_tl_types::types::EncryptedMessageService {
            random_id: 1,
            chat_id: id,
            date: 1,
            bytes: wire,
        }
        .into(),
        qts: 1,
    }
    .into();
    a.handle_update(&update).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(*order.lock().unwrap(), vec!["ChatClosed", "DecryptFailed"]);
    a.stop().await.unwrap();
}

async fn committed_mailbox(retry_down: bool) {
    let (a, store, world, id) = seeded::ready(Options::default()).await;
    if retry_down {
        world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
        assert!(matches!(
            a.send_message(id, "pending own send", None, None).await,
            Err(Error::SendPending { .. })
        ));
    }
    let wire = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &body("durable incoming", 0),
        Direction::Responder,
    )
    .unwrap();
    // Stop at the real commit-to-dispatch boundary, as if the process ended
    // after authenticated receipt but before invoking a consumer.
    a.protocol(|p| p.receive(id, &wire, None, 1)).unwrap();
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id].in_seq_no,
        1
    );
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .deliveries
            .len(),
        1
    );
    a.stop().await.unwrap();
    if retry_down {
        world
            .lock()
            .unwrap()
            .fail_sends_from
            .insert(1000, RpcFailure::Transient);
    }
    let recovered = SecretChatManager::new(
        support::Fake {
            user: 1000,
            world: Arc::clone(&world),
        },
        store.clone(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    recovered
        .on("MessageReceived", "collect", move |event| {
            if let Event::MessageReceived(message) = event {
                seen.lock().unwrap().push(message.text);
            }
            Ok(())
        })
        .unwrap();
    recovered.start().await.unwrap();
    assert_eq!(*got.lock().unwrap(), vec!["durable incoming"]);
    let chat = store.state.lock().unwrap().as_ref().unwrap().chats[&id].clone();
    assert!(chat.deliveries.is_empty());
    if retry_down {
        assert!(chat.outbox.values().any(|row| row.pending));
        let sent = world.lock().unwrap().sent.clone();
        assert!(sent.len() >= 2);
        assert_eq!(
            sent.first().unwrap().outgoing.frame,
            sent.last().unwrap().outgoing.frame
        );
        assert_eq!(
            sent.first().unwrap().outgoing.random_id,
            sent.last().unwrap().outgoing.random_id
        );
    }
    recovered.stop().await.unwrap();
}
#[tokio::test]
async fn committed_mailbox_restarts_after_dispatch_interruption() {
    review_9c5::concurrent_snapshot_keeps_flush_after_ordinary().await;
    committed_mailbox(false).await;
}
#[tokio::test]
async fn committed_mailbox_delivers_at_start_when_retained_send_still_fails() {
    committed_mailbox(true).await;
}
#[tokio::test]
async fn accepted_rekey_cannot_be_overwritten_by_another_request() {
    missing_dh_local_and_automatic_rekey_close_safely().await;
    let mut chat = seeded::chat();
    chat.exchange = Some(crate::chat::Exchange::Accepted {
        id: 100,
        key: SharedKey::from_slice(&[9; 256]).unwrap(),
    });
    chat.state = ChatState::Rekeying;
    let (a, store, world, id) = seeded::ready_chat(chat, Options::default()).await;
    let before =
        serde_json::to_value(store.state.lock().unwrap().as_ref().unwrap().chats[&id].clone())
            .unwrap();
    a.protocol(|p| {
        p.mutate_chat(id, |chat| {
            let mut effects = crate::protocol::Effects::default();
            assert!(!crate::rekey::handle(
                chat,
                &crate::types::DecryptedMessageActionRequestKey {
                    exchange_id: 200,
                    g_a: b"bad".to_vec(),
                }
                .into(),
                &mut effects,
                1
            )?);
            assert!(effects.sends.is_empty());
            Ok(())
        })
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(store.state.lock().unwrap().as_ref().unwrap().chats[&id].clone())
            .unwrap(),
        before
    );
    assert!(world.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn retry_without_original_wire_metadata_fails_closed_before_rpc() {
    queued_send_must_report_local_closure().await;
    closed_chat_does_not_launch_remaining_resend_effects().await;
    retained_wire_effects_are_serialized().await;
    let (a, store, world, id) = seeded::ready(Options::default()).await;
    let mut effect = a
        .protocol(|p| p.prepare_message(id, message("legacy"), None))
        .unwrap();
    effect.outgoing.frame.clear();
    assert!(matches!(
        a.transmit(effect, a.generation().unwrap()).await,
        Err(Error::ResendUnavailable)
    ));
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert!(world.lock().unwrap().sent.is_empty());
    assert!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .outbox
            .is_empty()
    );
    a.stop().await.unwrap();
}

async fn queued_send_must_report_local_closure() {
    let (manager, store, world, id) = seeded::ready(Options::default()).await;
    let wire = Arc::clone(manager.inner.wire.lock().unwrap().entry(id).or_default());
    let held = wire.lock().await;
    let sender = manager.clone();
    let pending = tokio::spawn(async move {
        sender
            .send_message(id, "queued before close", None, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.state.lock().unwrap().as_ref().unwrap().chats[&id]
            .outbox
            .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    manager.close(id).await.unwrap();
    drop(held);
    let result = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result,Err(Error::ChatClosed{chat_id}) if chat_id==id));
    assert!(world.lock().unwrap().sent.is_empty());
    manager.stop().await.unwrap();
}

async fn retained_wire_effects_are_serialized() {
    use std::{future::Future, task::Poll};
    let (a, _store, world, id) = seeded::ready(Options::default()).await;
    let effect = a
        .protocol(|p| p.prepare_message(id, message("one retained identity"), None))
        .unwrap();
    let expected = effect.outgoing.clone();
    let stale = effect.clone();
    let gate = Arc::new(support::Gate::default());
    world.lock().unwrap().send_gate = Some(gate);
    let mut first = Box::pin(a.transmit(effect, a.generation().unwrap()));
    std::future::poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let mut retry = Box::pin(a.retry_pending(id));
    std::future::poll_fn(|cx| {
        assert!(retry.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let maximum = world.lock().unwrap().maximum_sends;
    assert_eq!(
        maximum, 1,
        "a callback response and public retry must not overlap their same retained RPC"
    );
    drop(first); // An already issued RPC is uncertain; the retained identity remains.
    world.lock().unwrap().send_gate = None;
    retry.await.unwrap();
    let sent = world.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter().all(|row| row.outgoing.frame == expected.frame
            && row.outgoing.random_id == expected.random_id)
    );
    // A completed or already acknowledged row is not issued a second time from
    // an earlier detached effect snapshot.
    a.transmit(stale.clone(), a.generation().unwrap())
        .await
        .unwrap();
    assert_eq!(world.lock().unwrap().sent.len(), 2);
    a.protocol(|p| {
        p.mutate_chat(id, |chat| {
            chat.outbox.remove(&expected.seq_no);
            Ok(())
        })
    })
    .unwrap();
    a.transmit(stale, a.generation().unwrap()).await.unwrap();
    assert_eq!(world.lock().unwrap().sent.len(), 2);
    let withdrawn = a
        .protocol(|p| p.prepare_message(id, message("withdrawn stale effect"), None))
        .unwrap();
    a.delete_messages(id, [withdrawn.outgoing.random_id])
        .await
        .unwrap();
    let count = world.lock().unwrap().sent.len();
    a.transmit(withdrawn, a.generation().unwrap())
        .await
        .unwrap();
    assert_eq!(world.lock().unwrap().sent.len(), count);
    for effect in &world.lock().unwrap().sent[2..] {
        let body = crate::crypto::decrypt_frame(
            &SharedKey::from_slice(&[7; 256]).unwrap(),
            &effect.outgoing.frame,
            Direction::Initiator,
        )
        .unwrap();
        assert!(matches!(
            crate::framing::unwrap(&body).unwrap().message,
            crate::enums::DecryptedMessage::Service(_)
        ));
    }
    a.stop().await.unwrap();
}
async fn closed_chat_does_not_launch_remaining_resend_effects() {
    let (a, _store, world, id) = seeded::ready(Options::default()).await;
    a.send_message(id, "first", None, None).await.unwrap();
    a.send_message(id, "second", None, None).await.unwrap();
    let first_wire = world.lock().unwrap().sent[0].outgoing.seq_no;
    let last_wire = world.lock().unwrap().sent[1].outgoing.seq_no;
    let action = crate::types::DecryptedMessageService {
        random_id: 37,
        action: crate::types::DecryptedMessageActionResend {
            start_seq_no: first_wire,
            end_seq_no: last_wire,
        }
        .into(),
    };
    let wire = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &crate::framing::wrap(action.into(), 144, 1, 0)
            .unwrap()
            .to_bytes(),
        Direction::Responder,
    )
    .unwrap();
    let gate = Arc::new(support::Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let mut resending = tokio::spawn(async move { manager.receive(id, &wire, None, 1).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    a.close(id).await.unwrap();
    gate.release.notify_one();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(3), &mut resending).await;
    let received = match completed {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => panic!("the receive task must not panic"),
        Err(_) => {
            resending.abort();
            let _ = resending.await;
            panic!("the receive task must finish after local closure");
        }
    };
    assert!(
        matches!(received, Err(Error::SendPending { chat_id, cause: crate::error::PendingCause::Cancelled, .. }) if chat_id == id)
    );
    let sends = world.lock().unwrap().sent.len();
    assert_eq!(
        sends, 3,
        "the second detached resend must not begin after local closure"
    );
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    a.stop().await.unwrap();
}

async fn missing_dh_local_and_automatic_rekey_close_safely() {
    for automatic in [false, true] {
        let mut chat = seeded::chat();
        chat.dh = None;
        if automatic {
            chat.messages_since_rekey = crate::rekey::MESSAGE_TRIGGER + 1;
        }
        let (a, store, world, id) = seeded::ready_chat(chat, Options::default()).await;
        let closed = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&closed);
        a.on("ChatClosed", "missing_dh", move |event| {
            if let Event::ChatClosed { reason, .. } = event {
                assert_eq!(
                    reason,
                    "stored chat lacks its DH parameters and cannot rekey"
                );
                seen.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        })
        .unwrap();
        let result = if automatic {
            a.send_message(id, "requires rekey", None, None)
                .await
                .map(|_| ())
        } else {
            a.rekey(id).await
        };
        assert!(matches!(result, Err(Error::ChatClosed {chat_id}) if chat_id==id));
        assert_eq!(closed.load(Ordering::Acquire), 1);
        assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
        assert_eq!(
            store.state.lock().unwrap().as_ref().unwrap().chats[&id].closed_reason,
            Some(crate::error::CloseReason::MissingDh)
        );
        assert!(world.lock().unwrap().sent.is_empty());
        assert_eq!(world.lock().unwrap().discards, vec![(id, false)]);
        a.stop().await.unwrap();
    }
}
