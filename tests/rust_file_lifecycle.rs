//! Concurrent atomic file saves retain each other's active staging files.
#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::files::{FileKey, encrypt_file, save_file};
use std::{
    io::{Cursor, Read},
    sync::mpsc,
    time::Duration,
};
fn cipher() -> (FileKey, Vec<u8>) {
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let mut output = Vec::new();
    encrypt_file(&mut &[b'x'; 65][..], &mut output, 65, &key).unwrap();
    (key, output)
}
#[test]
fn changing_short_ciphertext_preserves_the_original_destination() {
    struct Short(Cursor<Vec<u8>>);
    impl Read for Short {
        fn read(&mut self, to: &mut [u8]) -> std::io::Result<usize> {
            if self.0.position() >= 16 {
                return Ok(0);
            }
            let count = to.len().min(16);
            self.0.read(&mut to[..count])
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.bin");
    std::fs::write(&path, b"ORIGINAL").unwrap();
    let (key, data) = cipher();
    assert!(
        save_file(
            &mut Short(Cursor::new(data)),
            &path,
            65,
            &key,
            key.fingerprint()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"ORIGINAL");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn a_competing_save_preserves_the_first_active_staging_file() {
    struct Held {
        input: Cursor<Vec<u8>>,
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }
    impl Read for Held {
        fn read(&mut self, to: &mut [u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            self.input.read(to)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.bin");
    let second = dir.path().join("second.bin");
    let (key, data) = cipher();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let one = key.clone();
        let bytes = data.clone();
        let target = first.clone();
        let pending = scope.spawn(move || {
            save_file(
                &mut Held {
                    input: Cursor::new(bytes),
                    entered: Some(entered_tx),
                    release: release_rx,
                },
                &target,
                65,
                &one,
                one.fingerprint(),
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let result = save_file(&mut &data[..], &second, 65, &key, key.fingerprint());
        release_tx.send(()).unwrap();
        result.unwrap();
        pending.join().unwrap().unwrap();
    });
    assert_eq!(std::fs::read(first).unwrap(), vec![b'x'; 65]);
    assert_eq!(std::fs::read(second).unwrap(), vec![b'x'; 65]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
