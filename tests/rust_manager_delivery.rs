#![allow(clippy::unwrap_used)]
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    Serializable,
    chat::Envelope,
    crypto::{Direction, SharedKey, decrypt_frame, encrypt_frame},
    enums::{DecryptedMessage, DecryptedMessageAction},
    manager::{Event, Options, SecretChatManager},
    protocol::Protocol,
    storage::{MemoryStorage, State, Storage},
    types,
};
use seeded::*;
use std::sync::{Arc, Mutex, atomic::Ordering};
use support::*;
fn body(raw: i32, ack: i32, text: &str) -> Vec<u8> {
    kiaro_secret_chat::framing::wrap(
        types::DecryptedMessage {
            no_webpage: false,
            silent: false,
            random_id: 37 + raw as i64,
            ttl: 0,
            message: text.into(),
            media: None,
            entities: None,
            via_bot_name: None,
            reply_to_random_id: None,
            grouped_id: None,
        }
        .into(),
        144,
        ack,
        raw * 2,
    )
    .unwrap()
    .to_bytes()
}
#[tokio::test]
async fn failed_receive_commit_advances_no_counter_and_emits_no_message() {
    acknowledgement_failure_retries_without_duplicate_callback().await;
    let (a, store, _w, id) = ready(Options::default()).await;
    let before = serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    a.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message.text);
        }
        Ok(())
    })
    .unwrap();
    store.failures.store(1, Ordering::Release);
    let frame = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &body(0, 1, "incoming"),
        Direction::Responder,
    )
    .unwrap();
    assert!(a.receive(id, &frame, None, 1).await.is_err());
    assert_eq!(
        serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap(),
        before
    );
    assert!(got.lock().unwrap().is_empty());
    a.stop().await.unwrap();
}
async fn acknowledgement_failure_retries_without_duplicate_callback() {
    let (manager, store, _, id) = ready(Options::default()).await;
    let observed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = observed.clone();
    let failed = store.failures.clone();
    manager
        .on("MessageReceived", "fail_ack", move |_| {
            if seen.fetch_add(1, Ordering::AcqRel) == 0 {
                failed.store(1, Ordering::Release);
            }
            Ok(())
        })
        .unwrap();
    let frame = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &body(0, 1, "observed once across acknowledgement retry"),
        Direction::Responder,
    )
    .unwrap();
    assert!(manager.receive(id, &frame, None, 1).await.is_err());
    assert_eq!(observed.load(Ordering::Acquire), 1);
    assert_eq!(
        store.load().unwrap().unwrap().chats[&id].deliveries.len(),
        1
    );
    manager.receive(id, &frame, None, 1).await.unwrap();
    assert!(
        store.load().unwrap().unwrap().chats[&id]
            .deliveries
            .is_empty(),
        "retry must settle the retained acknowledgement"
    );
    assert_eq!(
        observed.load(Ordering::Acquire),
        1,
        "ack retry must not replay the callback"
    );
    let services = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = services.clone();
    let failed = store.failures.clone();
    manager
        .on("ServiceActionReceived", "fail_service_ack", move |_| {
            if seen.fetch_add(1, Ordering::AcqRel) == 0 {
                failed.store(1, Ordering::Release);
            }
            Ok(())
        })
        .unwrap();
    let service = kiaro_secret_chat::framing::wrap(
        types::DecryptedMessageService {
            random_id: 91,
            action: types::DecryptedMessageActionSetMessageTtl { ttl_seconds: 42 }.into(),
        }
        .into(),
        144,
        1,
        2,
    )
    .unwrap()
    .to_bytes();
    let frame = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &service,
        Direction::Responder,
    )
    .unwrap();
    assert!(manager.receive(id, &frame, None, 1).await.is_err());
    assert_eq!(services.load(Ordering::Acquire), 1);
    assert_eq!(
        store.load().unwrap().unwrap().chats[&id]
            .service_deliveries
            .len(),
        1
    );
    manager.receive(id, &frame, None, 1).await.unwrap();
    assert!(
        store.load().unwrap().unwrap().chats[&id]
            .service_deliveries
            .is_empty()
    );
    assert_eq!(
        services.load(Ordering::Acquire),
        1,
        "service acknowledgement retry must not re-emit its callback"
    );
    manager.stop().await.unwrap();
}
#[tokio::test]
async fn peer_counter_advancement_acknowledges_each_random_id_once() {
    let (a, store, _w, id) = ready(Options::default()).await;
    let random = a.send_message(id, "ack me", None, None).await.unwrap();
    let acknowledgements = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&acknowledgements);
    a.on("MessageAcknowledged", "ack", move |event| {
        if let Event::MessageAcknowledged { random_ids, .. } = event {
            seen.lock().unwrap().push(random_ids);
        }
        Ok(())
    })
    .unwrap();
    let frame = encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &body(0, 3, "peer"),
        Direction::Responder,
    )
    .unwrap();
    a.receive(id, &frame, None, 1).await.unwrap();
    a.receive(id, &frame, None, 1).await.unwrap();
    assert_eq!(*acknowledgements.lock().unwrap(), vec![vec![random]]);
    assert!(store.load().unwrap().unwrap().chats[&id].outbox.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn rekey_confirmation_noop_uses_new_key_and_retires_old_key() {
    let (aa, bb) = Fake::pair();
    let w = Arc::clone(&aa.world);
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
    pump(&a, &b, &w).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &w).await;
    let old = store.load().unwrap().unwrap().chats[&id]
        .key
        .clone()
        .unwrap();
    let start = w.lock().unwrap().sent.len();
    a.rekey(id).await.unwrap();
    pump(&a, &b, &w).await;
    let after = store.load().unwrap().unwrap().chats[&id].clone();
    let key = after.key.as_ref().unwrap();
    assert_ne!(key.fingerprint(), old.fingerprint());
    let found=w.lock().unwrap().sent[start..].iter().any(|effect|decrypt_frame(key,&effect.outgoing.frame,Direction::Responder).ok().and_then(|body|kiaro_secret_chat::framing::unwrap(&body).ok()).is_some_and(|layer|matches!(layer.message,DecryptedMessage::Service(service)if matches!(service.action,DecryptedMessageAction::Noop))));
    assert!(found);
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
    assert!(after.previous_key.is_none());
    assert!(after.exchange.is_none());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn rejected_rekey_preparation_rolls_back_before_rpc() {
    let (a, store, w, id) = ready(Options::default()).await;
    let before = serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap();
    store.failures.store(1, Ordering::Release);
    assert!(a.rekey(id).await.is_err());
    assert_eq!(
        serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap(),
        before
    );
    assert!(w.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[test]
fn missing_predecessor_drains_the_full_gap_window() {
    let mut c = chat();
    for raw in 1..=kiaro_secret_chat::sequence::MAX_RESEND_COUNT {
        kiaro_secret_chat::sequence::accept(
            &mut c,
            Envelope {
                body: body(raw, 1, "queued"),
                file: None,
                date: 1,
            },
        )
        .unwrap();
    }
    assert_eq!(c.gaps.len(), 1000);
    let result = kiaro_secret_chat::sequence::accept(
        &mut c,
        Envelope {
            body: body(0, 1, "first"),
            file: None,
            date: 1,
        },
    )
    .unwrap();
    assert_eq!(result.ready.len(), 1001);
    assert!(c.gaps.is_empty());
    assert!(c.gap_end.is_none() && c.resend_due.is_none());
}
#[test]
fn gap_inspection_is_sorted_detached_and_writes_nothing() {
    let mut c = chat();
    c.gaps.insert(
        2,
        Envelope {
            body: body(2, 1, "two"),
            file: None,
            date: 1,
        },
    );
    c.gaps.insert(
        1,
        Envelope {
            body: body(1, 1, "one"),
            file: None,
            date: 1,
        },
    );
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(c.id, c);
    let mut store = TestStorage::default();
    store.commit(&state).unwrap();
    let protocol = Protocol::open(store.clone(), "11".repeat(32)).unwrap();
    let commits = store.commits.load(Ordering::Acquire);
    store.failures.store(1, Ordering::Release);
    let mut read = protocol.chat_record(7).unwrap().gaps;
    assert_eq!(read.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
    read.get_mut(&1).unwrap().body.push(255);
    assert_ne!(
        read[&1].body,
        protocol.chat_record(7).unwrap().gaps[&1].body
    );
    assert_eq!(store.commits.load(Ordering::Acquire), commits);
    assert_eq!(store.failures.load(Ordering::Acquire), 1);
}
