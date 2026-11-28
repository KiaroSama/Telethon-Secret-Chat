#![allow(clippy::unwrap_used)]
#[path = "support_manager/bounded.rs"]
mod bounded;
#[path = "support_manager/mod.rs"]
mod support;
use bounded::*;
use kiaro_secret_chat::Deserializable;
use kiaro_secret_chat::{
    error::Error,
    manager::{Options, SecretChatManager, SendFileOptions},
    media::FileSource,
    storage::{FileStorage, Operation},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::*;
#[tokio::test]
async fn explicit_pause_survives_file_reopen_until_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let (a, b, world, id) = pair(
        FileStorage::open(&path).unwrap(),
        limits(),
        Options::default(),
    )
    .await;
    let key = a.status(id).unwrap().key_fingerprint;
    assert_eq!(a.list().unwrap().len(), 1);
    a.pause_chat(id).unwrap();
    let before = world.lock().unwrap().sent.len();
    assert!(a.send_message(id, "refused", None, None).await.is_err());
    assert_eq!(world.lock().unwrap().sent.len(), before);
    a.stop().await.unwrap();
    let transport = Fake {
        user: 1000,
        world: Arc::clone(&world),
    };
    let resumed = SecretChatManager::new(
        transport,
        kiaro_secret_chat::custody::CustodyStorage::new(
            FileStorage::open(&path).unwrap(),
            "11".repeat(32),
            limits(),
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    resumed.start().await.unwrap();
    assert!(resumed.pause_status(id).unwrap().paused);
    assert_eq!(resumed.status(id).unwrap().key_fingerprint, key);
    resumed.resume_chat(id).await.unwrap();
    resumed
        .send_message(id, "continued", None, None)
        .await
        .unwrap();
    assert!(!resumed.pause_status(id).unwrap().paused);
    assert!(world.lock().unwrap().discards.is_empty());
    assert!(resumed.auto_save_secret_chats().unwrap().is_none());
    resumed.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn paused_conversation_leaves_other_peer_serviceable() {
    let (a, b, world, first) = pair(TestStorage::default(), limits(), Options::default()).await;
    let second = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(second).await.unwrap();
    pump(&a, &b, &world).await;
    a.pause_chat(first).unwrap();
    a.send_message(second, "healthy", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    b.send_message(second, "reply", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(a.pending_relay_deliveries(second).unwrap().len(), 1);
    assert!(a.pause_status(first).unwrap().paused);
    assert!(!a.pause_status(second).unwrap().paused);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn rejected_receive_keeps_existing_receipt_and_forbids_resume() {
    let allowed = Arc::new(AtomicBool::new(true));
    let admitted = Arc::clone(&allowed);
    let options = Options {
        admission: Some(Arc::new(move |_, operation| {
            operation != Operation::Receive || admitted.load(Ordering::Acquire)
        })),
        ..Default::default()
    };
    let store = TestStorage::default();
    let (a, b, world, id) = pair(store.clone(), limits(), options).await;
    b.send_message(id, "held", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    let before = a.pending_relay_deliveries(id).unwrap();
    let counter = store.state.lock().unwrap().as_ref().unwrap().chats[&id].in_seq_no;
    allowed.store(false, Ordering::Release);
    b.send_message(id, "rejected", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    let after = a.pending_relay_deliveries(id).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].message.body, before[0].message.body);
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id].in_seq_no,
        counter
    );
    assert!(!a.pause_status(id).unwrap().resumable);
    assert!(a.resume_chat(id).await.is_err());
    assert!(world.lock().unwrap().discards.is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn custody_age_pause_preserves_original_obligation() {
    partial_pause_failure_still_cancels_committed_chat().await;
    let mut quota = limits();
    quota.age_seconds = 0.2;
    let (a, b, world, id) = pair(TestStorage::default(), quota, Options::default()).await;
    b.send_message(id, "age", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    let before = a.pending_relay_deliveries(id).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    a.check_custody().unwrap();
    assert!(a.pause_status(id).unwrap().paused);
    let after = a.pending_relay_deliveries(id).unwrap();
    assert_eq!(after[0].seq_no, before[0].seq_no);
    assert_eq!(after[0].message.body, before[0].message.body);
    assert!(world.lock().unwrap().discards.is_empty());
    a.acknowledge_delivery(id, after[0].seq_no).unwrap();
    assert!(a.pending_relay_deliveries(id).unwrap().is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn unrelated_cloud_updates_consume_no_chat_capacity() {
    let mut quota = limits();
    quota.chats = 1;
    let (a, b, world, id) = pair(TestStorage::default(), quota, Options::default()).await;
    for _ in 0..3 {
        a.handle_update(&grammers_tl_types::types::UpdatePtsChanged {}.into())
            .await;
    }
    assert_eq!(a.list().unwrap().len(), 1);
    a.send_message(id, "still healthy", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.read_history(id, 1).unwrap()[0].text, "still healthy");
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
async fn paused_effect(operation: &str) {
    let (a, b, world, id) = pair(TestStorage::default(), limits(), Options::default()).await;
    b.send_file(
        id,
        FileSource::bytes(b"fixture".to_vec(), "sample.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    let reference = a.pending_relay_deliveries(id).unwrap()[0].message.clone();
    let envelope = kiaro_secret_chat::framing::unwrap(&reference.body).unwrap();
    let media = match envelope.message {
        kiaro_secret_chat::enums::DecryptedMessage::Message(m) => m.media.unwrap(),
        _ => panic!("expected media"),
    };
    let attached =
        grammers_tl_types::enums::EncryptedFile::from_bytes(reference.file.as_ref().unwrap())
            .unwrap();
    let attached = match attached {
        grammers_tl_types::enums::EncryptedFile::File(file) => file,
        _ => panic!("expected attached file"),
    };
    let reference = kiaro_secret_chat::media::MediaReference::new(id, &attached, &media).unwrap();
    a.pause_chat(id).unwrap();
    let before = {
        let w = world.lock().unwrap();
        (
            w.sent.len(),
            w.uploads,
            w.accepts.len(),
            w.request_ids.len(),
        )
    };
    let result = match operation {
        "accept" => a.accept(id).await.map(|_| ()),
        "retry" => a.retry_pending(id).await,
        "rekey" => a.rekey(id).await,
        "ttl" => a.set_ttl(id, 3).await,
        "read" => a.mark_read(id, vec![1]).await,
        "screenshot" => a.screenshot(id, vec![1]).await,
        "typing" => a.set_typing(id, None).await,
        "file" => a
            .send_file(
                id,
                FileSource::bytes(vec![1], "sample.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
            .map(|_| ()),
        "forward" => a.forward_file(id, &reference, "", None).await.map(|_| ()),
        _ => panic!("unknown operation"),
    };
    assert!(matches!(result, Err(Error::Paused)));
    {
        let w = world.lock().unwrap();
        assert_eq!(
            (
                w.sent.len(),
                w.uploads,
                w.accepts.len(),
                w.request_ids.len()
            ),
            before
        );
    }
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
macro_rules! paused {
    ($name:ident,$operation:literal) => {
        #[tokio::test]
        async fn $name() {
            paused_effect($operation).await;
        }
    };
}
paused!(paused_accept_sends_nothing, "accept");
paused!(paused_retry_sends_nothing, "retry");
paused!(paused_rekey_sends_nothing, "rekey");
paused!(paused_ttl_sends_nothing, "ttl");
paused!(paused_read_sends_nothing, "read");
paused!(paused_screenshot_sends_nothing, "screenshot");
paused!(paused_typing_sends_nothing, "typing");
paused!(paused_upload_sends_nothing, "file");
paused!(paused_forward_sends_nothing, "forward");

async fn partial_pause_failure_still_cancels_committed_chat() {
    use std::sync::{Mutex, atomic::AtomicUsize};
    let storage = TestStorage::default();
    let mut quota = limits();
    quota.age_seconds = 2.0;
    let (a, b, world, first) = pair(storage.clone(), quota, Options::default()).await;
    let second = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(second).await.unwrap();
    pump(&a, &b, &world).await;
    assert!(first < second);
    let entered = Arc::new(AtomicUsize::new(0));
    let running = entered.clone();
    let cleaned = Arc::new(Mutex::new(Vec::new()));
    let done = cleaned.clone();
    a.on_async_with_cleanup(
        "MessageReceived",
        "custody_failure",
        move |_| {
            let entered = running.clone();
            async move {
                entered.fetch_add(1, Ordering::AcqRel);
                std::future::pending::<kiaro_secret_chat::error::Result<()>>().await
            }
        },
        move |event| {
            let done = done.clone();
            async move {
                if let kiaro_secret_chat::manager::Event::MessageReceived(message) = event {
                    done.lock().unwrap().push(message.chat_id);
                }
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(first, "first owed", None, None)
        .await
        .unwrap();
    b.send_message(second, "second owed", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = a.pending_relay_deliveries(first).unwrap();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    storage.fail_on_commit.store(
        storage.commits.load(Ordering::Acquire) + 2,
        Ordering::Release,
    );
    assert!(matches!(a.check_custody(), Err(Error::Storage)));
    assert!(a.pause_status(first).unwrap().paused);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), async {
            while !cleaned.lock().unwrap().contains(&first) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok(),
        "a later failed pause must not skip cancellation of the already committed paused chat"
    );
    assert_eq!(
        storage.state.lock().unwrap().as_ref().unwrap().chats[&first]
            .deliveries
            .values()
            .next()
            .unwrap()
            .body,
        before[0].message.body
    );
    storage.fail_on_commit.store(0, Ordering::Release);
    a.check_custody().unwrap();
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
