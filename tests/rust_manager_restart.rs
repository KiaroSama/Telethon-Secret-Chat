#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    manager::{Event, Options, SecretChatManager},
    storage::{FileStorage, MemoryStorage, State},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use support::*;
type DurableManager = SecretChatManager<FileStorage, Fake>;
fn durable(fake: Fake, path: &Path) -> DurableManager {
    SecretChatManager::new(
        fake,
        FileStorage::open(path).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap()
}
async fn pair(path: &Path) -> (DurableManager, Manager, Fake, Arc<Mutex<World>>, i32) {
    let (a, b) = Fake::pair();
    let world = Arc::clone(&a.world);
    let transport = a.clone();
    let a = durable(a, path);
    let b = SecretChatManager::new(
        b,
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
    (a, b, transport, world, id)
}
fn persisted(path: &Path) -> State {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
#[tokio::test]
async fn key_counters_layer_and_role_survive_real_file_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (a, b, transport, world, id) = pair(&path).await;
    for text in ["one", "two"] {
        a.send_message(id, text, Some(vec![]), None).await.unwrap();
    }
    pump(&a, &b, &world).await;
    let before = persisted(&path).chats.remove(&id).unwrap();
    a.stop().await.unwrap();
    // The stopped owner deliberately remains alive while a fresh file owner opens.
    let revived = durable(transport, &path);
    revived.start().await.unwrap();
    let after = persisted(&path).chats.remove(&id).unwrap();
    assert_eq!(
        serde_json::to_value(&after.key).unwrap(),
        serde_json::to_value(&before.key).unwrap()
    );
    assert_eq!(
        after.key.as_ref().unwrap().fingerprint(),
        before.key.as_ref().unwrap().fingerprint()
    );
    assert_eq!(
        (
            after.in_seq_no,
            after.out_seq_no,
            after.capability_layer,
            after.is_outbound
        ),
        (
            before.in_seq_no,
            before.out_seq_no,
            before.capability_layer,
            before.is_outbound
        )
    );
    assert_eq!(after.state, ChatState::Ready);
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn conversation_continues_in_order_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (a, b, transport, world, id) = pair(&path).await;
    let texts = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&texts);
    b.on("MessageReceived", "texts", move |event| {
        if let Event::MessageReceived(m) = event {
            sink.lock().unwrap().push(m.text);
        }
        Ok(())
    })
    .unwrap();
    a.send_message(id, "before the restart", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    a.stop().await.unwrap();
    let revived = durable(transport, &path);
    revived.start().await.unwrap();
    revived
        .send_message(id, "after the restart", None, None)
        .await
        .unwrap();
    pump(&revived, &b, &world).await;
    assert_eq!(
        *texts.lock().unwrap(),
        ["before the restart", "after the restart"]
    );
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn reopened_manager_lists_every_stored_chat() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (a, b, transport, _, id) = pair(&path).await;
    a.stop().await.unwrap();
    let revived = durable(transport, &path);
    revived.start().await.unwrap();
    assert_eq!(
        revived
            .list()
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [id]
    );
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn closed_tombstone_stays_closed_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (a, b, transport, _, id) = pair(&path).await;
    a.close(id).await.unwrap();
    a.stop().await.unwrap();
    let revived = durable(transport, &path);
    revived.start().await.unwrap();
    assert_eq!(revived.status(id).unwrap().state, ChatState::Closed);
    assert!(revived.send_message(id, "late", None, None).await.is_err());
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn initial_outbound_request_resumes_before_acceptance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = durable(aa.clone(), &path);
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
    a.stop().await.unwrap();
    let revived = durable(aa, &path);
    revived.start().await.unwrap();
    b.accept(id).await.unwrap();
    pump(&revived, &b, &world).await;
    assert_eq!(revived.status(id).unwrap().state, ChatState::Ready);
    assert_eq!(
        revived.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
    let chat = persisted(&path).chats.remove(&id).unwrap();
    assert!(chat.handshake_secret.is_none());
    assert!(chat.peer_public.is_none());
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn pending_request_is_announced_again_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("b.json");
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = SecretChatManager::new(
        aa,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = durable(bb.clone(), &path);
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.stop().await.unwrap();
    let revived = durable(bb, &path);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    revived
        .on("ChatRequested", "announce", move |event| {
            if let Event::ChatRequested {
                chat_id,
                peer_user_id,
            } = event
            {
                sink.lock().unwrap().push((chat_id, peer_user_id));
            }
            Ok(())
        })
        .unwrap();
    revived.start().await.unwrap();
    assert_eq!(*seen.lock().unwrap(), [(id, 1000)]);
    revived.accept(id).await.unwrap();
    pump(&a, &revived, &world).await;
    assert_eq!(revived.status(id).unwrap().state, ChatState::Ready);
    revived.stop().await.unwrap();
    a.stop().await.unwrap();
}
#[tokio::test]
async fn archive_switch_and_explicit_off_survive_file_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (transport, _) = Fake::pair();
    let first = durable(transport.clone(), &path);
    first.start().await.unwrap();
    assert_eq!(
        first.auto_save_secret_chats().unwrap(),
        Some(directory.path().join("saved-secret-chats"))
    );
    let saved = directory.path().join("saved");
    first.start_auto_save_secret_chats(&saved).await.unwrap();
    first.stop().await.unwrap();
    let second = durable(transport.clone(), &path);
    second.start().await.unwrap();
    assert_eq!(second.auto_save_secret_chats().unwrap(), Some(saved));
    second.stop_auto_save_secret_chats().await.unwrap();
    second.stop().await.unwrap();
    let third = durable(transport, &path);
    third.start().await.unwrap();
    assert!(third.auto_save_secret_chats().unwrap().is_none());
    third.stop().await.unwrap();
}
#[tokio::test]
async fn default_archive_continues_same_chat_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state").join("chats.json");
    let (a, b, transport, world, id) = pair(&path).await;
    assert_eq!(
        a.auto_save_secret_chats().unwrap(),
        Some(path.parent().unwrap().join("saved-secret-chats"))
    );
    a.send_message(id, "before restart", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let before = a.status(id).unwrap().key_fingerprint;
    let sequence = persisted(&path).chats[&id].out_seq_no;
    a.stop().await.unwrap();
    let revived = durable(transport, &path);
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    revived
        .on("MessageReceived", "collect", move |event| {
            if let Event::MessageReceived(m) = event {
                sink.lock().unwrap().push(m.text);
            }
            Ok(())
        })
        .unwrap();
    revived.start().await.unwrap();
    assert_eq!(revived.status(id).unwrap().key_fingerprint, before);
    revived
        .send_message(id, "after restart", None, None)
        .await
        .unwrap();
    b.send_message(id, "peer after restart", None, None)
        .await
        .unwrap();
    pump(&revived, &b, &world).await;
    assert_eq!(*got.lock().unwrap(), ["peer after restart"]);
    assert!(persisted(&path).chats[&id].out_seq_no > sequence);
    let texts = revived
        .read_saved_messages(id)
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        ["before restart", "after restart", "peer after restart"]
    );
    assert!(world.lock().unwrap().discards.is_empty());
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn default_archive_downloads_files_without_explicit_enable() {
    use kiaro_secret_chat::{manager::SendFileOptions, media::FileSource};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (a, b, _transport, world, id) = pair(&path).await;
    let content = b"synthetic auto-saved document".to_vec();
    b.send_file(
        id,
        FileSource::bytes(content.clone(), "sample.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    a.stop_auto_save_secret_chats().await.unwrap();
    let records = a.read_saved_messages(id).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        std::fs::read(records[0]["file"].as_str().unwrap()).unwrap(),
        content
    );
    assert!(
        !directory
            .path()
            .join("saved-secret-chats")
            .join(id.to_string())
            .join("pending.json")
            .exists()
    );
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn off_before_first_file_start_stays_off_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (transport, bb) = Fake::pair();
    let world = Arc::clone(&transport.world);
    let first = durable(transport.clone(), &path);
    first.stop_auto_save_secret_chats().await.unwrap();
    first.start().await.unwrap();
    first.stop().await.unwrap();
    let second = durable(transport, &path);
    let peer_manager = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    second.start().await.unwrap();
    peer_manager.start().await.unwrap();
    let id = second.create(peer(2000)).await.unwrap().id;
    pump(&second, &peer_manager, &world).await;
    peer_manager.accept(id).await.unwrap();
    pump(&second, &peer_manager, &world).await;
    second
        .send_message(id, "not archived", None, None)
        .await
        .unwrap();
    assert!(second.auto_save_secret_chats().unwrap().is_none());
    assert!(!directory.path().join("saved-secret-chats").exists());
    second.stop().await.unwrap();
    peer_manager.stop().await.unwrap();
}
#[tokio::test]
async fn relative_state_and_archive_keep_original_directory_after_cwd_change() {
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }
    let previous = std::env::current_dir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let _restore = Restore(previous);
    let original = directory.path().join("original");
    let elsewhere = directory.path().join("elsewhere");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&elsewhere).unwrap();
    std::env::set_current_dir(&original).unwrap();
    let storage = FileStorage::open("state.json").unwrap();
    assert_eq!(storage.path(), original.join("state.json"));
    let (transport, _) = Fake::pair();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    std::env::set_current_dir(&elsewhere).unwrap();
    manager.start().await.unwrap();
    assert_eq!(
        manager.auto_save_secret_chats().unwrap(),
        Some(original.join("saved-secret-chats"))
    );
    assert!(!elsewhere.join("state.json").exists());
    assert_eq!(
        persisted(&original.join("state.json")).settings["auto_save_secret_chats"]["on"],
        true
    );
    manager.stop().await.unwrap();
}
#[tokio::test]
async fn memory_archive_disabled_preference_beats_new_default_folder() {
    let directory = tempfile::tempdir().unwrap();
    let chosen = directory.path().join("chosen");
    let different = directory.path().join("different");
    let (transport, _) = Fake::pair();
    let store = TestStorage::default();
    let first = SecretChatManager::new(
        transport.clone(),
        store.clone(),
        "11".repeat(32),
        Options {
            auto_save_folder: Some(chosen.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    first.start().await.unwrap();
    assert_eq!(
        first.auto_save_secret_chats().unwrap(),
        Some(chosen.clone())
    );
    first.stop_auto_save_secret_chats().await.unwrap();
    first.stop().await.unwrap();
    let second = SecretChatManager::new(
        transport,
        store.clone(),
        "11".repeat(32),
        Options {
            auto_save_folder: Some(different),
            ..Default::default()
        },
    )
    .unwrap();
    second.start().await.unwrap();
    assert!(second.auto_save_secret_chats().unwrap().is_none());
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().settings["auto_save_secret_chats"]["folder"],
        chosen.to_str().unwrap()
    );
    second.stop().await.unwrap();
}
#[tokio::test]
async fn failed_initial_archive_preference_does_not_start_recovery_effects() {
    use std::sync::atomic::Ordering;
    let directory = tempfile::tempdir().unwrap();
    let (transport, _) = Fake::pair();
    let world = Arc::clone(&transport.world);
    let store = TestStorage::default();
    let failure = Arc::clone(&store.failures);
    let manager = SecretChatManager::new(
        transport,
        store,
        "11".repeat(32),
        Options {
            auto_save_folder: Some(directory.path().join("chosen")),
            ..Default::default()
        },
    )
    .unwrap();
    failure.store(1, Ordering::Release);
    assert!(manager.start().await.is_err());
    assert!(manager.auto_save_secret_chats().unwrap().is_none());
    assert!(world.lock().unwrap().request_ids.is_empty());
    assert!(world.lock().unwrap().sent.is_empty());
    manager.start().await.unwrap();
    assert!(manager.auto_save_secret_chats().unwrap().is_some());
    manager.stop().await.unwrap();
}
