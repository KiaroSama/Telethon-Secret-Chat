#![allow(clippy::unwrap_used)]
#[path = "support_manager/logging.rs"]
mod logging;
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    error::Error,
    manager::{Event, Options, RpcFailure, SecretChatManager, SendFileOptions},
    media::FileSource,
    storage::Storage,
};
use seeded::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use support::*;
#[tokio::test]
async fn legacy_pending_request_without_secret_closes_at_start() {
    let mut record = chat();
    record.state = ChatState::Requested;
    record.key = None;
    record.handshake_secret = None;
    record.peer_public = None;
    let (a, _store, _w, id) = construct(record, Options::default());
    let reasons = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&reasons);
    a.on("ChatClosed", "collect", move |event| {
        if let Event::ChatClosed { reason, .. } = event {
            seen.lock().unwrap().push(reason);
        }
        Ok(())
    })
    .unwrap();
    a.start().await.unwrap();
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert_eq!(
        *reasons.lock().unwrap(),
        vec!["legacy pending handshake has no recoverable secret"]
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn startup_reports_unsent_durable_work_without_raw_failure() {
    let (a, store, w, id) = ready(Options::default()).await;
    w.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending { random_id, .. } =
        a.send_message(id, "stuck", None, None).await.unwrap_err()
    else {
        panic!("expected pending identity");
    };
    a.stop().await.unwrap();
    w.lock()
        .unwrap()
        .fail_sends
        .insert(random_id, RpcFailure::Transient);
    let capture = logging::Capture::default();
    let logs = Arc::clone(&capture.0);
    let _guard = tracing::subscriber::set_default(capture);
    let resumed = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        store,
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    resumed.start().await.unwrap();
    assert!(
        logs.lock()
            .unwrap()
            .join("\n")
            .contains("durable work awaiting retry")
    );
    assert_eq!(
        w.lock().unwrap().sent.last().unwrap().outgoing.random_id,
        random_id
    );
    resumed.stop().await.unwrap();
}
#[tokio::test]
async fn accepting_without_peer_public_value_refuses_before_secret_or_rpc() {
    let mut record = chat();
    record.state = ChatState::Waiting;
    record.key = None;
    record.handshake_secret = None;
    record.peer_public = None;
    let (a, store, w, id) = ready_chat(record, Options::default()).await;
    assert!(matches!(a.accept(id).await,Err(Error::ChatNotReady{chat_id})if chat_id==id));
    assert!(
        store.load().unwrap().unwrap().chats[&id]
            .handshake_secret
            .is_none()
    );
    assert!(w.lock().unwrap().accepts.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn peer_delete_and_flush_update_runtime_history_in_order() {
    let (a, b, w, id) = established().await;
    let first = a.send_message(id, "one", None, None).await.unwrap();
    a.send_message(id, "two", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(
        b.read_history(id, 10)
            .unwrap()
            .into_iter()
            .map(|message| message.text)
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    a.delete_messages(id, vec![first]).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(
        b.read_history(id, 10)
            .unwrap()
            .into_iter()
            .map(|message| message.text)
            .collect::<Vec<_>>(),
        vec!["two"]
    );
    a.flush_history(id).await.unwrap();
    pump(&a, &b, &w).await;
    assert!(b.read_history(id, 10).unwrap().is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn saving_text_or_negative_media_metadata_creates_no_destination() {
    let (a, b, w, id) = established().await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    b.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message);
        }
        Ok(())
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    a.send_message(id, "just text", None, None).await.unwrap();
    pump(&a, &b, &w).await;
    let text = events.lock().unwrap().last().unwrap().clone();
    assert!(
        b.save_message_file(&text, &dir.path().join("text.bin"))
            .await
            .is_err()
    );
    a.send_file(
        id,
        FileSource::bytes(b"content".to_vec(), "file.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &w).await;
    let mut message = *events.lock().unwrap().last().unwrap().clone();
    let Some(kiaro_secret_chat::enums::DecryptedMessageMedia::Document(media)) =
        message.media.as_mut()
    else {
        panic!("expected document");
    };
    media.size = -1;
    assert!(
        b.save_message_file(&message, &dir.path().join("bad.bin"))
            .await
            .is_err()
    );
    assert_eq!(w.lock().unwrap().downloads, 0);
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn synchronous_callback_completes_in_place() {
    let (a, _store, _w, id) = ready(Options::default()).await;
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    a.on("ChatClosed", "inline", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    a.close(id).await.unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);
    assert_eq!(a.active_handler_tasks(), 0);
    a.stop().await.unwrap();
}
