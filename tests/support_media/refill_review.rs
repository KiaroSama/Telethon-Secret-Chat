#![allow(clippy::unwrap_used)]
use super::*;
use std::{sync::mpsc, time::Duration};
impl SourceReader {
    pub(crate) async fn verify_refill_ownership() {
        run().await;
    }
}
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
struct Pausing<R> {
    reader: R,
    first: bool,
    entered: Arc<tokio::sync::Notify>,
    release: mpsc::Receiver<()>,
    finished: Option<mpsc::Sender<()>>,
}
impl<R: Read> Read for Pausing<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.first {
            self.first = false;
            let count = buffer.len().min(1);
            let read = self.reader.read(&mut buffer[..count])?;
            self.entered.notify_one();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(read)
        } else {
            let read = self.reader.read(buffer)?;
            if let Some(finished) = self.finished.take() {
                finished.send(()).unwrap();
            }
            Ok(read)
        }
    }
}
pub(super) async fn run() {
    let mut source = FileSource::bytes(vec![3; 32], "partial.bin")
        .unwrap()
        .open()
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let (send, receive) = mpsc::channel();
    let release = Release(Some(send));
    let (finished, completion) = mpsc::channel();
    let read = Pausing {
        reader: source.reader.refill_reader(),
        first: true,
        entered: entered.clone(),
        release: receive,
        finished: Some(finished),
    };
    let key = crate::files::FileKey::new(&[7; 32], &[9; 32]).unwrap();
    let mut encrypted = crate::media::EncryptingReader::new(read, source.size, key)
        .unwrap()
        .with_coordination(source.reader.coordination());
    let mut buffer = [0; 32];
    let mut reading = Box::pin(tokio::io::AsyncReadExt::read_exact(
        &mut encrypted,
        &mut buffer,
    ));
    tokio::select! { result=&mut reading=>panic!("refill did not pause: {result:?}"),_=entered.notified()=>{} }
    drop(reading);
    drop(encrypted);
    let (rewound, observed) = mpsc::channel();
    let rewind = std::thread::spawn(move || {
        source.rewind().unwrap();
        rewound.send(()).unwrap();
        source
    });
    let escaped = observed.recv_timeout(Duration::from_millis(100)).is_ok();
    drop(release);
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut source = rewind.join().unwrap();
    let position = source.reader().stream_position().unwrap();
    assert!(
        !escaped,
        "rewind completed while a cancelled refill still owned more reads"
    );
    assert_eq!(position, 0, "the old refill read after rewind completed");
    eprintln!("Cancelled native refill ownership and synchronous rewind control passed");
}
