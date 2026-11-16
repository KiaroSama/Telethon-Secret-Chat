#![allow(clippy::unwrap_used)]
#[path = "support_manager/bounded.rs"]
mod bounded;
#[path = "support_manager/mod.rs"]
mod support;
use bounded::*;
use kiaro_secret_chat::{
    Deserializable,
    enums::DecryptedMessage,
    manager::{Options, SendFileOptions},
    media::{FileSource, MediaReference},
};
use std::{sync::Arc, time::Duration};
use support::*;
async fn source() -> (
    Bounded<TestStorage>,
    Manager,
    Arc<std::sync::Mutex<World>>,
    i32,
    MediaReference,
    Vec<u8>,
) {
    let (a, b, w, id) = pair(TestStorage::default(), limits(), Options::default()).await;
    let content = b"bounded media fixture\0".repeat(100);
    b.send_file(
        id,
        FileSource::bytes(content.clone(), "sample.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &w).await;
    let held = a.pending_relay_deliveries(id).unwrap();
    let body = kiaro_secret_chat::framing::unwrap(&held[0].message.body).unwrap();
    let media = match body.message {
        DecryptedMessage::Message(m) => m.media.unwrap(),
        _ => panic!("expected file"),
    };
    let file =
        grammers_tl_types::enums::EncryptedFile::from_bytes(held[0].message.file.as_ref().unwrap())
            .unwrap();
    let file = match file {
        grammers_tl_types::enums::EncryptedFile::File(file) => file,
        _ => panic!("expected attached file"),
    };
    let reference = MediaReference::new(id, &file, &media).unwrap();
    (a, b, w, id, reference, content)
}
#[tokio::test]
async fn capped_receive_refuses_before_download_and_creates_no_file() {
    let (a, b, w, id, reference, content) = source().await;
    let dir = tempfile::tempdir().unwrap();
    assert!(
        a.receive_file_with_limit(&reference, content.len() - 1)
            .await
            .is_err()
    );
    assert_eq!(w.lock().unwrap().downloads, 0);
    assert_eq!(
        a.receive_file_with_limit(&reference, content.len())
            .await
            .unwrap(),
        content
    );
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    assert!(a.auto_save_secret_chats().unwrap().is_none());
    assert!(!a.pause_status(id).unwrap().paused);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
async fn damaged_download(oversize: bool) {
    let (a, b, w, id, reference, content) = source().await;
    let dir = tempfile::tempdir().unwrap();
    let original = w.lock().unwrap().files[&reference.file_id].clone();
    w.lock().unwrap().files.insert(
        reference.file_id,
        vec![0; if oversize { original.len() + 16 } else { 1 }],
    );
    assert!(
        a.receive_file_with_limit(&reference, content.len())
            .await
            .is_err()
    );
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    assert!(a.auto_save_secret_chats().unwrap().is_none());
    w.lock().unwrap().files.insert(reference.file_id, original);
    assert_eq!(
        a.receive_file_with_limit(&reference, content.len())
            .await
            .unwrap(),
        content
    );
    assert!(!a.pause_status(id).unwrap().paused);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn oversized_ciphertext_is_refused_and_releases_capacity() {
    damaged_download(true).await;
}
#[tokio::test]
async fn truncated_ciphertext_is_refused_and_releases_capacity() {
    damaged_download(false).await;
}
#[tokio::test]
async fn cancelled_ram_download_releases_capacity_without_disk() {
    let (a, b, w, id, reference, content) = source().await;
    let gate = Arc::new(Gate::default());
    w.lock().unwrap().download_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let target = reference.clone();
    let task = tokio::spawn(async move {
        manager
            .receive_file_with_limit(&target, content.len())
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    w.lock().unwrap().download_gate = None;
    let bytes = a
        .receive_file_with_limit(&reference, 1_000_000)
        .await
        .unwrap();
    assert_eq!(bytes, b"bounded media fixture\0".repeat(100));
    assert!(!a.pause_status(id).unwrap().paused);
    assert!(a.auto_save_secret_chats().unwrap().is_none());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
