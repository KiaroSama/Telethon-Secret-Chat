#![allow(clippy::unwrap_used)]
#[path = "support_manager/logging.rs"]
mod logging;
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    Deserializable, Serializable,
    chat::{ChatState, Exchange},
    crypto::{Direction, SharedKey, decrypt_frame, encrypt_frame},
    enums::{DecryptedMessage, DecryptedMessageAction},
    error::Error,
    manager::{Event, Options, SecretChatManager, SendFileOptions},
    media::FileSource,
    storage::{MemoryStorage, Storage},
    types,
};
use seeded::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use support::*;
fn message(text: &str) -> DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 37,
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
fn frame(inner: DecryptedMessage, raw: i32) -> Vec<u8> {
    encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &kiaro_secret_chat::framing::wrap(inner, 144, 1, raw * 2)
            .unwrap()
            .to_bytes(),
        Direction::Responder,
    )
    .unwrap()
}
fn update(id: i32, wire: Vec<u8>) -> grammers_tl_types::enums::Update {
    grammers_tl_types::types::UpdateNewEncryptedMessage {
        message: grammers_tl_types::types::EncryptedMessage {
            random_id: 37,
            chat_id: id,
            date: 1,
            bytes: wire,
            file: grammers_tl_types::enums::EncryptedFile::Empty,
        }
        .into(),
        qts: 1,
    }
    .into()
}
async fn resend(a: &Seeded, id: i32, seq: i32) {
    let service = types::DecryptedMessageService {
        random_id: 71,
        action: types::DecryptedMessageActionResend {
            start_seq_no: seq,
            end_seq_no: seq,
        }
        .into(),
    };
    a.handle_update(&update(id, frame(service.into(), 0))).await;
}
#[tokio::test]
async fn forged_pending_fingerprint_never_switches_key() {
    let mut c = chat();
    c.exchange = Some(Exchange::Accepted {
        id: 123,
        key: SharedKey::from_slice(&[9; 256]).unwrap(),
    });
    c.state = ChatState::Rekeying;
    let (a, store, _w, id) = ready_chat(c, Options::default()).await;
    let before = serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap();
    let mut forged = SharedKey::from_slice(&[9; 256])
        .unwrap()
        .fingerprint()
        .to_le_bytes()
        .to_vec();
    forged.extend_from_slice(&[0; 64]);
    a.handle_update(&update(id, forged)).await;
    assert_eq!(
        serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap(),
        before
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn failed_send_commit_rolls_back_chat_and_retention() {
    let (a, store, w, id) = ready(Options::default()).await;
    let before = serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap();
    store.failures.store(1, Ordering::Release);
    assert!(
        a.send_message(id, "must never get a sequence hole", None, None)
            .await
            .is_err()
    );
    let after = store.load().unwrap().unwrap().chats[&id].clone();
    assert_eq!(serde_json::to_value(&after).unwrap(), before);
    assert!(after.outbox.is_empty());
    assert!(w.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn outer_random_id_equals_authenticated_inner_id() {
    let (a, _store, w, id) = ready(Options::default()).await;
    let random = a.send_message(id, "identity", None, None).await.unwrap();
    let outgoing = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    let body = decrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &outgoing.frame,
        Direction::Initiator,
    )
    .unwrap();
    assert_eq!(outgoing.random_id, random);
    assert_eq!(
        kiaro_secret_chat::framing::unwrap(&body)
            .unwrap()
            .message
            .random_id(),
        random
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn sequenced_service_selects_service_transport() {
    let (a, _store, w, id) = ready(Options::default()).await;
    a.set_ttl(id, 60).await.unwrap();
    let effect = w.lock().unwrap().sent.last().unwrap().clone();
    assert!(effect.outgoing.service);
    assert!(effect.outgoing.file.is_none());
    assert!(
        matches!(kiaro_secret_chat::framing::unwrap(&effect.outgoing.body).unwrap().message,DecryptedMessage::Service(s)if matches!(s.action,DecryptedMessageAction::SetMessageTtl(_)))
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn peer_resend_reuses_confirmed_ciphertext_sequence_and_id() {
    let (a, store, w, id) = ready(Options::default()).await;
    a.send_message(id, "retained", None, None).await.unwrap();
    let first = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert!(!store.load().unwrap().unwrap().chats[&id].outbox[&first.seq_no].pending);
    resend(&a, id, first.seq_no).await;
    let replay = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert_eq!(replay.frame, first.frame);
    assert_eq!(replay.random_id, first.random_id);
    assert_eq!(replay.seq_no, first.seq_no);
    assert_eq!(store.load().unwrap().unwrap().chats[&id].out_seq_no, 1);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn close_scrubs_keys_queues_and_history() {
    let (a, store, _w, id) = ready(Options::default()).await;
    a.send_message(id, "retained", None, None).await.unwrap();
    a.handle_update(&update(id, frame(message("stored"), 0)))
        .await;
    assert_eq!(a.read_history(id, 1).unwrap().len(), 1);
    a.close(id).await.unwrap();
    let c = store.load().unwrap().unwrap().chats[&id].clone();
    assert_eq!(c.state, ChatState::Closed);
    assert!(
        c.key.is_none()
            && c.previous_key.is_none()
            && c.exchange.is_none()
            && c.handshake_secret.is_none()
            && c.peer_public.is_none()
    );
    assert!(
        c.outbox.is_empty()
            && c.gaps.is_empty()
            && c.deliveries.is_empty()
            && c.service_deliveries.is_empty()
    );
    assert!(a.read_history(id, 100).unwrap().is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn documented_closed_event_alias_runs_once() {
    let (a, _store, _w, id) = ready(Options::default()).await;
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    a.on("ChatClosed", "collect", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    a.close(id).await.unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);
    a.stop().await.unwrap();
}
struct Sensitive(Arc<AtomicUsize>);
impl std::fmt::Debug for Sensitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fetch_add(1, Ordering::AcqRel);
        f.write_str("SYNTHETIC-CREDENTIAL-IN-REPR")
    }
}
async fn private_handler(debug: bool) {
    let capture = logging::Capture::default();
    let logs = Arc::clone(&capture.0);
    let _guard = tracing::subscriber::set_default(capture);
    let (a, _store, _w, id) = ready(Options::default()).await;
    let representations = Arc::new(AtomicUsize::new(0));
    let sensitive = Sensitive(Arc::clone(&representations));
    a.on("MessageReceived", "failure", move |_| {
        let _ = &sensitive;
        Err(if debug {
            Error::Media
        } else {
            Error::MediaKind {
                kind: "SYNTHETIC-SECRET-NOT-FOR-LOGGING".into(),
            }
        })
    })
    .unwrap();
    a.handle_update(&update(id, frame(message("inbound"), 0)))
        .await;
    let logs = logs.lock().unwrap().join("\n");
    assert!(!logs.contains("SYNTHETIC-SECRET-NOT-FOR-LOGGING"));
    assert!(!logs.contains("SYNTHETIC-CREDENTIAL-IN-REPR"));
    assert!(logs.contains("handler failed"));
    assert_eq!(representations.load(Ordering::Acquire), 0);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn handler_failure_never_logs_error_payload() {
    private_handler(false).await;
}
#[tokio::test]
async fn handler_debug_never_logs_callable_representation() {
    private_handler(true).await;
}
#[tokio::test]
async fn requesting_zero_history_returns_empty() {
    let (a, _store, _w, id) = ready(Options::default()).await;
    a.handle_update(&update(id, frame(message("stored"), 0)))
        .await;
    assert_eq!(a.read_history(id, 1).unwrap().len(), 1);
    assert!(a.read_history(id, 0).unwrap().is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn invalid_ttl_changes_no_state_or_transport() {
    let (a, store, w, id) = ready(Options::default()).await;
    let before = serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap();
    assert!(a.set_ttl(id, -1).await.is_err());
    assert_eq!(
        serde_json::to_value(store.load().unwrap().unwrap().chats[&id].clone()).unwrap(),
        before
    );
    assert!(w.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn delete_iterator_is_consumed_once() {
    let (a, b, w, id) = established().await;
    let actions = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&actions);
    b.on("ServiceActionReceived", "collect", move |event| {
        if let Event::ServiceActionReceived {
            action: DecryptedMessageAction::DeleteMessages(action),
            ..
        } = event
        {
            seen.lock().unwrap().push(action.random_ids);
        }
        Ok(())
    })
    .unwrap();
    let consumed = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&consumed);
    let ids = [11, 22].into_iter().inspect(move |_| {
        called.fetch_add(1, Ordering::AcqRel);
    });
    a.delete_messages(id, ids).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(consumed.load(Ordering::Acquire), 2);
    assert_eq!(actions.lock().unwrap().last().unwrap(), &vec![11, 22]);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn rekey_commit_is_authenticated_with_previous_key() {
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
    let new = store.load().unwrap().unwrap().chats[&id]
        .key
        .clone()
        .unwrap();
    let mut commits = Vec::new();
    for effect in &w.lock().unwrap().sent[start..] {
        for (is_old, key) in [(true, &old), (false, &new)] {
            if let Ok(body) = decrypt_frame(key, &effect.outgoing.frame, Direction::Initiator)
                && let Ok(layer) = kiaro_secret_chat::framing::unwrap(&body)
                && matches!(layer.message,DecryptedMessage::Service(s)if matches!(s.action,DecryptedMessageAction::CommitKey(_)))
            {
                commits.push(is_old);
            }
        }
    }
    assert_eq!(commits, vec![true]);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn gap_keeps_each_files_original_server_envelope() {
    let (a, b, w, id) = established().await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&received);
    b.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message);
        }
        Ok(())
    })
    .unwrap();
    a.send_file(
        id,
        FileSource::bytes(b"FIRST".to_vec(), "one.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    a.send_file(
        id,
        FileSource::bytes(b"SECOND".to_vec(), "two.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    let (first, second) = {
        let mut w = w.lock().unwrap();
        let updates = w.updates.get_mut(&2000).unwrap();
        (updates.pop_front().unwrap(), updates.pop_front().unwrap())
    };
    b.handle_update(&second).await;
    b.handle_update(&first).await;
    let messages = received.lock().unwrap().clone();
    assert_eq!(messages.len(), 2);
    assert_ne!(
        messages[0].file.as_ref().unwrap().id,
        messages[1].file.as_ref().unwrap().id
    );
    let dir = tempfile::tempdir().unwrap();
    let path = b
        .save_message_file(&messages[1], &dir.path().join("received.bin"))
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"SECOND");
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
fn file_id(file: &grammers_tl_types::enums::InputEncryptedFile) -> i64 {
    match file {
        grammers_tl_types::enums::InputEncryptedFile::File(f) => f.id,
        grammers_tl_types::enums::InputEncryptedFile::Uploaded(f) => f.id,
        grammers_tl_types::enums::InputEncryptedFile::BigUploaded(f) => f.id,
        _ => panic!("expected file handle"),
    }
}
#[tokio::test]
async fn peer_file_resend_preserves_media_transport_and_server_id() {
    let (a, _store, w, id) = ready(Options::default()).await;
    a.send_file(
        id,
        FileSource::bytes(b"retained media".to_vec(), "file.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    let original = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    resend(&a, id, original.seq_no).await;
    let repeated = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert!(!repeated.service);
    assert_eq!(repeated.frame, original.frame);
    let first =
        grammers_tl_types::enums::InputEncryptedFile::from_bytes(original.file.as_ref().unwrap())
            .unwrap();
    let second =
        grammers_tl_types::enums::InputEncryptedFile::from_bytes(repeated.file.as_ref().unwrap())
            .unwrap();
    assert_eq!(file_id(&first), file_id(&second));
    a.stop().await.unwrap();
}
