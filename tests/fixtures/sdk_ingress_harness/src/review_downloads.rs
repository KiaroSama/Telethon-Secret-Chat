//! A real slow filesystem bounds download RPC progress and queued chunk bytes.
use kiaro_secret_chat::{
    grammers_client::Client,
    grammers_mtsender::SenderPool,
    grammers_session::{SessionData, storages::MemorySession},
    grammers_tl_types::{self as tl, Identifiable, Serializable},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::Notify;
pub fn bounded_chunk_progress() {
    oversized_encrypted_chunk_is_refused_before_disk_write();
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut data = SessionData {
                home_dc: 2,
                ..Default::default()
            };
            data.dc_options.get_mut(&2).unwrap().auth_key = Some([7; 256]);
            let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
            let client = Client::new(pool.handle.clone());
            let handle = pool.handle.clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let first = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let overrun = Arc::new(Notify::new());
            let progress = Arc::new(Notify::new());
            let count = calls.clone();
            let initial = first.clone();
            let resume = release.clone();
            let too_many = overrun.clone();
            let advanced = progress.clone();
            let runner = tokio::spawn(pool.runner.run_offline_fixture(move |_, body| {
                let count = count.clone();
                let initial = initial.clone();
                let resume = resume.clone();
                let too_many = too_many.clone();
                let advanced = advanced.clone();
                async move {
                    assert_eq!(
                        u32::from_le_bytes(body[..4].try_into().unwrap()),
                        tl::functions::upload::GetFile::CONSTRUCTOR_ID
                    );
                    let nth = count.fetch_add(1, Ordering::AcqRel) + 1;
                    if nth == 1 {
                        initial.notify_one();
                        resume.notified().await;
                    }
                    if nth == 8 {
                        advanced.notify_one();
                    }
                    if nth == 11 {
                        too_many.notify_one();
                    }
                    Ok(tl::enums::upload::File::File(tl::types::upload::File {
                        r#type: tl::enums::storage::FileType::FileUnknown,
                        mtime: 0,
                        bytes: vec![42; 512 * 1024],
                    })
                    .to_bytes())
                }
            }));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("large.bin");
            let destination = path.clone();
            let download = tokio::spawn(async move {
                client
                    .download_media(
                        &super::File {
                            dc: 2,
                            size: 16 * 1024 * 1024,
                        },
                        destination,
                    )
                    .await
            });
            tokio::time::timeout(Duration::from_secs(2), first.notified())
                .await
                .unwrap();
            let (unlock, locked) = mpsc::channel();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let occupied = tokio::task::spawn_blocking(move || {
                entered_tx.send(()).unwrap();
                locked.recv().unwrap();
            });
            entered_rx.await.unwrap();
            release.notify_one();
            // Four queued chunks + four producers + at most two Tokio writer buffers.
            tokio::time::timeout(Duration::from_secs(2), progress.notified())
                .await
                .unwrap();
            let unbounded = tokio::time::timeout(Duration::from_secs(1), overrun.notified())
                .await
                .is_ok();
            unlock.send(()).unwrap();
            occupied.await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), download)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(
                !unbounded,
                "RPC producers ran beyond the bounded slow-writer capacity"
            );
            assert_eq!(calls.load(Ordering::Acquire), 32);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(bytes.len(), 16 * 1024 * 1024);
            assert!(bytes.iter().all(|byte| *byte == 42));
            handle.quit();
            runner.await.unwrap();
            drop(pool.updates);
        });
    })
    .join()
    .unwrap();
}

// A blocked disk worker distinguishes pre-write refusal from eventual decrypt rejection.
fn oversized_encrypted_chunk_is_refused_before_disk_write() {
    std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap()
            .block_on(async {
                let mut data = SessionData {
                    home_dc: 2,
                    ..Default::default()
                };
                data.dc_options.get_mut(&2).unwrap().auth_key = Some([7; 256]);
                let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
                let client = Client::new(pool.handle.clone());
                let entered = Arc::new(Notify::new());
                let release = Arc::new(Notify::new());
                let called = entered.clone();
                let resume = release.clone();
                let runner = tokio::spawn(pool.runner.run_offline_fixture(move |_, _| {
                    let called = called.clone();
                    let resume = resume.clone();
                    async move {
                        called.notify_one();
                        resume.notified().await;
                        Ok(tl::enums::upload::File::File(tl::types::upload::File {
                            r#type: tl::enums::storage::FileType::FileUnknown,
                            mtime: 0,
                            bytes: vec![42; 32],
                        })
                        .to_bytes())
                    }
                }));
                let key = kiaro_secret_chat::files::FileKey::new(&[4; 32], &[8; 32]).unwrap();
                let reference = kiaro_secret_chat::media::MediaReference::new(
                    7,
                    &tl::types::EncryptedFile {
                        id: 2,
                        access_hash: 88,
                        size: 1,
                        dc_id: 2,
                        key_fingerprint: key.fingerprint(),
                    },
                    &kiaro_secret_chat::types::DecryptedMessageMediaDocument {
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
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("oversized.bin");
                let output = path.clone();
                let mut download = tokio::spawn(async move {
                    kiaro_secret_chat::media::download_file(&client, &reference, &output).await
                });
                tokio::time::timeout(Duration::from_secs(2), entered.notified())
                    .await
                    .unwrap();
                let (unlock, locked) = mpsc::channel();
                let (ready, waiting) = tokio::sync::oneshot::channel();
                let occupied = tokio::task::spawn_blocking(move || {
                    ready.send(()).unwrap();
                    locked.recv().unwrap();
                });
                waiting.await.unwrap();
                release.notify_one();
                let early = tokio::time::timeout(Duration::from_millis(200), &mut download).await;
                let refused_before_write =
                    matches!(early, Ok(Ok(Err(kiaro_secret_chat::error::Error::Media))));
                unlock.send(()).unwrap();
                occupied.await.unwrap();
                if early.is_err() {
                    let _ = download.await.unwrap();
                }
                pool.handle.quit();
                runner.await.unwrap();
                assert!(!path.exists());
                assert!(
                    refused_before_write,
                    "oversized encrypted chunk was queued for disk I/O before refusal"
                );
            });
    })
    .join()
    .unwrap();
}
