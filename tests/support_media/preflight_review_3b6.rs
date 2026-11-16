use super::*;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
#[derive(Clone, Copy)]
enum Probe {
    Open,
    Header,
}
struct GatedSource {
    bytes: Cursor<Vec<u8>>,
    probe: Probe,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    worker_entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    dropped: Arc<AtomicBool>,
}
impl GatedSource {
    fn gate(&mut self) {
        if let Some(entered) = self.entered.take() {
            self.worker_entered.send(()).unwrap();
            entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
    }
}
impl Read for GatedSource {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if matches!(self.probe, Probe::Header) {
            self.gate();
        }
        self.bytes.read(output)
    }
}
impl Seek for GatedSource {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        if matches!(self.probe, Probe::Open) && matches!(position, SeekFrom::End(0)) {
            self.gate();
        }
        self.bytes.seek(position)
    }
}
impl Drop for GatedSource {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}
pub async fn opening_seek_is_owned() {
    responsive_probe(Probe::Open).await;
    queued_options_are_charged().await;
}
pub async fn inferred_ogg_probe_is_owned() {
    responsive_probe(Probe::Header).await;
}
pub async fn protected_name_checks_keep_error_wrapper() {
    use kiaro_secret_chat::{custody::TransientLimits, error::Error, storage::Storage};
    for empty_name in [true, false] {
        let store = TestStorage::default();
        let limits = TransientLimits {
            bytes: 65536,
            frame_bytes: 4096,
            ..Default::default()
        };
        let (manager, peer, world, id) = transient::pair(store.clone(), limits).await;
        let before = store.load().unwrap().unwrap().chats[&id].out_seq_no;
        let error = manager
            .send_file(
                id,
                FileSource::Bytes {
                    data: vec![7; 65536],
                    name: if empty_name {
                        String::new()
                    } else {
                        "oversized.bin".into()
                    },
                },
                SendFileOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Paused),
            "Protected mode must retain its error-normalization contract for invalid or oversized bytes"
        );
        let snapshot = store.load().unwrap().unwrap();
        assert_eq!(snapshot.chats[&id].out_seq_no, before);
        assert!(snapshot.chats[&id].outbox.is_empty());
        assert_eq!(world.lock().unwrap().uploads, 0);
        manager.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}
async fn queued_options_are_charged() {
    use kiaro_secret_chat::{custody::TransientLimits, error::Error, storage::Storage};
    struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    let store = TestStorage::default();
    let limits = TransientLimits {
        bytes: 1024 * 1024,
        frame_bytes: 4096,
        ..Default::default()
    };
    let (manager, peer, world, id) = transient::pair(store.clone(), limits).await;
    let before = store.load().unwrap().unwrap().chats[&id].out_seq_no;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (worker_tx, _worker_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release_tx));
    let dropped = Arc::new(AtomicBool::new(false));
    let source = GatedSource {
        bytes: Cursor::new(vec![1]),
        probe: Probe::Open,
        entered: Some(entered_tx),
        worker_entered: worker_tx,
        release: release_rx,
        dropped: dropped.clone(),
    };
    let first_manager = manager.clone();
    let first = tokio::spawn(async move {
        first_manager
            .send_file(
                id,
                FileSource::stream(Box::new(source), "first.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();
    let second_manager = manager.clone();
    let mut second = tokio::spawn(async move {
        second_manager
            .send_file(
                id,
                FileSource::bytes(vec![2], "second.bin").unwrap(),
                SendFileOptions {
                    metadata: Metadata {
                        thumbnail: Some(vec![7; 16 * 1024 * 1024]),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
    });
    let result = tokio::time::timeout(Duration::from_secs(1), &mut second).await;
    let refused_while_queued = matches!(&result, Ok(Ok(Err(Error::Paused))));
    if result.is_err() {
        second.abort();
        let _ = second.await;
    }
    let retained_before_release = !dropped.load(Ordering::Acquire);
    first.abort();
    let _ = first.await;
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), manager.stop())
        .await
        .unwrap()
        .unwrap();
    peer.stop().await.unwrap();
    assert!(
        refused_while_queued,
        "known options must exceed the content budget before the earlier opening gate releases"
    );
    assert!(retained_before_release);
    assert!(dropped.load(Ordering::Acquire));
    let snapshot = store.load().unwrap().unwrap();
    assert_eq!(snapshot.chats[&id].out_seq_no, before);
    assert!(snapshot.chats[&id].outbox.is_empty());
    assert_eq!(world.lock().unwrap().uploads, 0);
}
async fn responsive_probe(probe: Probe) {
    let (manager, peer, world, id) = established().await;
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
    let dropped = Arc::new(AtomicBool::new(false));
    let mut bytes = Cursor::new(b"HEADERpublic source preflight payload".to_vec());
    bytes.set_position(6);
    let source = GatedSource {
        bytes,
        probe,
        entered: Some(entered_tx),
        worker_entered: worker_tx,
        release: release_rx,
        dropped: dropped.clone(),
    };
    let sender = manager.clone();
    let sending = tokio::spawn(async move {
        sender
            .send_file(
                id,
                FileSource::stream(
                    Box::new(source),
                    match probe {
                        Probe::Open => "source.bin",
                        Probe::Header => "source.ogg",
                    },
                )
                .unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    entered_rx.await.unwrap();
    let mut overtook = false;
    let queued = if matches!(probe, Probe::Open) {
        let gate = Arc::new(Gate::default());
        world.lock().unwrap().upload_gate = Some(gate.clone());
        let second = manager.clone();
        let task = tokio::spawn(async move {
            second
                .send_file(
                    id,
                    FileSource::bytes(b"later file".to_vec(), "second.bin").unwrap(),
                    SendFileOptions::default(),
                )
                .await
        });
        overtook = tokio::time::timeout(Duration::from_secs(1), gate.entered.notified())
            .await
            .is_ok();
        Some((task, gate))
    } else {
        None
    };
    peer.send_message(id, "updates remain responsive during preflight", None, None)
        .await
        .unwrap();
    pump(&manager, &peer, &world).await;
    assert_eq!(
        manager.read_history(id, 1).unwrap()[0].text,
        "updates remain responsive during preflight"
    );
    let _ = progress_tx.send(());
    sending.abort();
    let _ = sending.await;
    if let Some((task, gate)) = queued {
        if !overtook {
            tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
                .await
                .unwrap();
        }
        task.abort();
        let _ = task.await;
        world.lock().unwrap().upload_gate = None;
    }
    let retained = !dropped.load(Ordering::Acquire);
    let mut stopping = Box::pin(manager.stop());
    let first = std::future::poll_fn(|cx| {
        Poll::Ready(match stopping.as_mut().poll(cx) {
            Poll::Pending => None,
            Poll::Ready(result) => Some(result),
        })
    })
    .await;
    let stopped_early = first.is_some();
    let _ = finish_tx.send(());
    if let Some(result) = first {
        result.unwrap();
    } else {
        tokio::time::timeout(Duration::from_secs(3), stopping)
            .await
            .unwrap()
            .unwrap();
    }
    let responsive = observer.join().unwrap();
    peer.stop().await.unwrap();
    assert!(
        responsive,
        "source Seek/OGG Read must not block the async update executor"
    );
    assert!(
        !overtook,
        "a later fast file must not overtake an admitted source preflight"
    );
    assert!(
        retained && !stopped_early,
        "cancellation must retain and join the actual source preflight worker"
    );
    assert!(dropped.load(Ordering::Acquire));
    assert_eq!(world.lock().unwrap().uploads, 0);
}
