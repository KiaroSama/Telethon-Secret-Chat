use kiaro_secret_chat::{files::FileKey, media::EncryptingReader};
use std::{
    io::{Cursor, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
struct GatedReader {
    inner: Cursor<Vec<u8>>,
    entered: Arc<AtomicBool>,
    started: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
}
impl Read for GatedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if let Some(started) = self.started.take() {
            self.entered.store(true, Ordering::Release);
            started.send(()).unwrap();
            self.release.recv().unwrap();
        }
        self.inner.read(bytes)
    }
}
pub fn asynchronous_source_does_not_block_executor() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (heartbeat_tx, heartbeat_rx) = mpsc::channel();
    let entered = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&entered);
    let observer = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let responsive = heartbeat_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        release_tx.send(()).unwrap();
        responsive
    });
    let key = FileKey::new(&[7; 32], &[9; 32]).unwrap();
    let source = GatedReader {
        inner: Cursor::new(vec![42; 31]),
        entered,
        started: Some(started_tx),
        release: release_rx,
    };
    let mut reader = EncryptingReader::new(source, 31, key.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let bytes = runtime.block_on(async move {
        let heartbeat = tokio::spawn(async move {
            while !observed.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
            let _ = heartbeat_tx.send(());
        });
        let mut bytes = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
            .await
            .unwrap();
        heartbeat.await.unwrap();
        bytes
    });
    assert!(
        observer.join().unwrap(),
        "an asynchronous encrypted source read must leave the executor responsive"
    );
    let mut plaintext = Vec::new();
    kiaro_secret_chat::files::decrypt_file(&mut bytes.as_slice(), &mut plaintext, 31, &key)
        .unwrap();
    assert_eq!(plaintext, vec![42; 31]);
}
