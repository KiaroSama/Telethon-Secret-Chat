use super::*;
use std::{
    future::Future,
    io::{Cursor, Read, Seek, SeekFrom},
    sync::mpsc,
    task::Poll,
};

struct SlowCopy {
    bytes: Cursor<Vec<u8>>,
    read: bool,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    worker_entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Read for SlowCopy {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.read
            && self.bytes.position() == 0
            && let Some(entered) = self.entered.take()
        {
            self.worker_entered.send(()).unwrap();
            entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        self.read = true;
        self.bytes.read(output)
    }
}
impl Seek for SlowCopy {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(position)
    }
}
pub async fn deletion_invalidates_sent_copy() {
    struct Release(Option<mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    let (manager, peer, _, id, folder) = saving().await;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (worker_tx, _worker_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Release(Some(release_tx));
    let source = SlowCopy {
        bytes: Cursor::new(b"public deleted sent attachment".to_vec()),
        read: false,
        entered: Some(entered_tx),
        worker_entered: worker_tx,
        release: release_rx,
    };
    let sending = manager.clone();
    let task = tokio::spawn(async move {
        sending
            .send_file(
                id,
                FileSource::stream(Box::new(source), "deleted.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();
    let deleted = manager.delete_saved_messages(id);
    let absent_after_delete = !folder.path().join(id.to_string()).exists();
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    deleted.unwrap();
    assert!(
        absent_after_delete && !folder.path().join(id.to_string()).exists(),
        "a retained sent-file worker must not recreate a deleted archive"
    );
    assert!(manager.read_saved_messages(id).unwrap().is_empty());
    manager.stop().await.unwrap();
    peer.stop().await.unwrap();
}
pub async fn copy_deadline_reports_committed_identity() {
    use kiaro_secret_chat::{
        chat::{Chat, ChatState, DhRecord},
        crypto::SharedKey,
        custody::{CustodyLimits, CustodyStorage},
        error::{Error, PendingCause},
        manager::{Options, SecretChatManager},
        storage::Storage,
    };
    struct Release(Option<mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    let store = TestStorage::default();
    let mut chat = Chat::new(7, 88, 2000, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: prime(),
    });
    chat.capability_layer = 144;
    chat.wrapper_layer = 144;
    chat.rekeyed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(chat.outbox.is_empty() && chat.messages_since_rekey == 0);
    let (transport, _) = Fake::pair();
    let world = transport.world.clone();
    world
        .lock()
        .unwrap()
        .requests
        .insert(7, (1000, 2000, vec![]));
    let limits = CustodyLimits {
        operation_seconds: 1.0,
        tasks: 4,
        frame_bytes: 8192,
        ..CustodyLimits::new(64, 1024 * 1024, 3600.0).unwrap()
    };
    let mut storage = CustodyStorage::new(store.clone(), "11".repeat(32), limits).unwrap();
    let mut state = storage.load().unwrap().unwrap();
    state.chats.insert(7, chat);
    storage.commit(&state).unwrap();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    let folder = tempfile::tempdir().unwrap();
    manager
        .start_auto_save_secret_chats(folder.path())
        .await
        .unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (worker_tx, _worker_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Release(Some(release_tx));
    let source = SlowCopy {
        bytes: Cursor::new(b"public archive deadline identity".to_vec()),
        read: false,
        entered: Some(entered_tx),
        worker_entered: worker_tx,
        release: release_rx,
    };
    let sender = manager.clone();
    let task = tokio::spawn(async move {
        sender
            .send_file(
                7,
                FileSource::stream(Box::new(source), "pending.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let snapshot = store.load().unwrap().unwrap();
    let pending = snapshot.chats[&7]
        .outbox
        .values()
        .filter(|row| row.pending && !row.service)
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), 1);
    let original = pending[0].clone();
    let returned_identity = matches!(error,Error::SendPending{chat_id:7,random_id,cause:PendingCause::Deadline} if random_id==original.random_id);
    assert!(
        world.lock().unwrap().sent.is_empty(),
        "no earlier pending send or due rekey may account for this deadline"
    );
    drop(release);
    manager.resume_chat(7).await.unwrap();
    {
        let sent = world.lock().unwrap();
        assert_eq!(sent.uploads, 1);
        assert_eq!(
            sent.sent.len(),
            1,
            "recovery retries the one committed file without creating a second file message"
        );
        assert_eq!(sent.sent[0].outgoing.random_id, original.random_id);
        assert_eq!(sent.sent[0].outgoing.frame, original.frame);
    }
    assert_eq!(store.load().unwrap().unwrap().chats[&7].out_seq_no, 1);
    manager.stop().await.unwrap();
    assert!(
        returned_identity,
        "archive-copy deadline must return the exact committed SendPending identity"
    );
}
pub async fn slow_copy_is_owned_without_blocking_executor() {
    let (manager, peer, _, id, _folder) = saving().await;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (worker_tx, worker_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let observer = std::thread::spawn(move || {
        worker_rx.recv().unwrap();
        let responsive = progress_rx.recv_timeout(Duration::from_secs(3)).is_ok();
        if responsive {
            finish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        release_tx.send(()).unwrap();
        responsive
    });
    let source = SlowCopy {
        bytes: Cursor::new(b"public slow saved-source fixture".to_vec()),
        read: false,
        entered: Some(entered_tx),
        worker_entered: worker_tx,
        release: release_rx,
    };
    let sender = manager.clone();
    let sending = tokio::spawn(async move {
        sender
            .send_file(
                id,
                FileSource::stream(Box::new(source), "slow.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    entered_rx.await.unwrap();
    // Both this runtime and an unrelated archive operation must progress while
    // the caller's synchronous source remains gated in its second pass.
    assert!(manager.read_saved_messages(id + 1).unwrap().is_empty());
    let _ = progress_tx.send(());
    sending.abort();
    let _ = sending.await;
    let mut stop = Box::pin(manager.stop());
    let first = std::future::poll_fn(|cx| {
        Poll::Ready(match stop.as_mut().poll(cx) {
            Poll::Pending => None,
            Poll::Ready(result) => Some(result),
        })
    })
    .await;
    let joined_before_release = first.is_some();
    let _ = finish_tx.send(());
    if let Some(result) = first {
        result.unwrap();
    } else {
        tokio::time::timeout(Duration::from_secs(3), stop)
            .await
            .unwrap()
            .unwrap();
    }
    let responsive = observer.join().unwrap();
    let rows = manager.read_saved_messages(id).unwrap();
    peer.stop().await.unwrap();
    assert!(
        responsive,
        "a synchronous archive copy must not block its async executor"
    );
    assert!(
        !joined_before_release,
        "stop must retain and join the blocked copy owner"
    );
    assert!(
        rows.iter().all(|row| row.get("file").is_none()),
        "a cancelled saved copy must not publish after stop"
    );
}
pub async fn cancelled_archive_stop_keeps_download_owner() {
    let (manager, peer, world, id, _folder) = saving().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().download_gate = Some(gate.clone());
    peer.send_file(
        id,
        FileSource::bytes(b"accepted owned download".to_vec(), "owned.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&manager, &peer, &world).await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    let mut stopping = Box::pin(manager.stop_auto_save_secret_chats());
    std::future::poll_fn(|cx| {
        assert!(stopping.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(stopping);
    manager.stop().await.unwrap();
    world.lock().unwrap().download_gate = None;
    gate.release.notify_waiters();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        world.lock().unwrap().downloads,
        0,
        "a cancelled stop waiter cannot detach an accepted download past manager shutdown"
    );
    assert!(
        manager
            .read_saved_messages(id)
            .unwrap()
            .iter()
            .all(|row| row.get("file").is_none())
    );
    peer.stop().await.unwrap();
}
pub async fn recovery_respects_configured_task_limit() {
    use kiaro_secret_chat::{
        autosave::AutoSave,
        custody::{CustodyLimits, CustodyStorage},
        files::{FileKey, encrypt_file},
        manager::{Options, SecretChatManager},
        media::MediaReference,
        types,
    };
    let folder = tempfile::tempdir().unwrap();
    let (archive, _) = AutoSave::enabled(folder.path()).unwrap();
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let plaintext = b"bounded recovery attachment";
    let mut encrypted = Vec::new();
    encrypt_file(
        &mut plaintext.as_slice(),
        &mut encrypted,
        plaintext.len() as u64,
        &key,
    )
    .unwrap();
    let reference = MediaReference::new(
        7,
        &grammers_tl_types::types::EncryptedFile {
            id: 77,
            access_hash: 88,
            size: encrypted.len() as i64,
            dc_id: 2,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: plaintext.len() as i64,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    for random in 0..20 {
        archive.owe(7, random, &reference, "recovered.bin").unwrap();
    }
    let (transport, _) = Fake::pair();
    let world = transport.world.clone();
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().download_gate = Some(gate.clone());
    world.lock().unwrap().files.insert(77, encrypted);
    let limits = CustodyLimits {
        tasks: 2,
        ..CustodyLimits::new(64, 1024 * 1024, 3600.0).unwrap()
    };
    let storage = CustodyStorage::new(TestStorage::default(), "11".repeat(32), limits).unwrap();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    manager
        .start_auto_save_secret_chats(folder.path())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while world.lock().unwrap().download_starts < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let admitted = world.lock().unwrap().download_starts;
    world.lock().unwrap().download_gate = None;
    gate.release.notify_waiters();
    tokio::time::timeout(
        Duration::from_secs(5),
        manager.stop_auto_save_secret_chats(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        admitted, 2,
        "recovery must queue excess obligations behind the configured task budget"
    );
    assert_eq!(world.lock().unwrap().downloads, 20);
    assert!(archive.pending(7).unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(folder.path().join("7/files"))
            .unwrap()
            .count(),
        20
    );
    manager.stop().await.unwrap();
}
