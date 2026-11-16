use super::*;
pub(super) async fn queued_settlement_refuses_stopped_runner() {
    use crate::{
        autosave::AutoSave,
        custody::{CustodyLimits, CustodyStorage},
        files::FileKey,
        media::MediaReference,
    };
    let folder = tempfile::tempdir().unwrap();
    let (archive, _) = AutoSave::enabled(folder.path()).unwrap();
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let reference = MediaReference::new(
        7,
        &grammers_tl_types::types::EncryptedFile {
            id: 77,
            access_hash: 88,
            size: 16,
            dc_id: 2,
            key_fingerprint: key.fingerprint(),
        },
        &crate::types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: 1,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    archive.owe(7, 91, &reference, "queued.bin").unwrap();
    let (transport, _) = crate::test_support::support::Fake::pair();
    let world = transport.world.clone();
    let limits = CustodyLimits {
        tasks: 1,
        ..CustodyLimits::new(64, 1024 * 1024, 3600.0).unwrap()
    };
    let storage = CustodyStorage::new(
        crate::test_support::support::TestStorage::default(),
        "11".repeat(32),
        limits,
    )
    .unwrap();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    let held = manager.inner.work.acquire(1).unwrap();
    manager
        .queue_saved_download(archive.clone(), 7, 91, reference, "queued.bin".into())
        .unwrap();
    manager.fence_runtime();
    drop(held);
    let runtime = tokio::runtime::Handle::current();
    let waiting = manager.clone();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let result = runtime.block_on(waiting.settle_saved_downloads(false));
        let _ = done_tx.send(result);
    });
    let result = tokio::time::timeout(Duration::from_secs(1), done_rx).await;
    // Always clear and join, including an old implementation busy-spinning on
    // the other thread; a failed regression must never leave an orphan behind.
    manager.stop().await.unwrap();
    thread.join().unwrap();
    assert!(
        matches!(result, Ok(Ok(Err(Error::ManagerStopping)))),
        "a stranded queue must refuse settlement instead of spinning"
    );
    assert_eq!(
        archive.pending(7).unwrap().len(),
        1,
        "stopping must retain the durable download obligation"
    );
    assert_eq!(world.lock().unwrap().download_starts, 0);
}
