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
