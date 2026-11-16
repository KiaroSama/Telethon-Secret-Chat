#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    Serializable, enums,
    files::{FileKey, decrypt_file},
    manager::{Event, MessageReceived, SendFileOptions},
    media::{self, FileSource, Metadata},
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};
use support::*;
fn collected(manager: &Manager) -> Arc<Mutex<Vec<MessageReceived>>> {
    let records = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&records);
    manager
        .on("MessageReceived", "collect_sources", move |event| {
            if let Event::MessageReceived(message) = event {
                sink.lock().unwrap().push(*message);
            }
            Ok(())
        })
        .unwrap();
    records
}
#[tokio::test]
async fn path_bytes_and_stream_have_identical_shape_and_plaintext() {
    let (a, b, world, id) = established().await;
    let got = collected(&b);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("clip.mp4");
    let content = (0..3000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    std::fs::write(&path, &content).unwrap();
    for source in [
        FileSource::path(&path),
        FileSource::bytes(content.clone(), "clip.mp4").unwrap(),
        FileSource::stream(Box::new(Cursor::new(content.clone())), "clip.mp4").unwrap(),
    ] {
        a.send_file(
            id,
            source,
            SendFileOptions {
                metadata: Metadata {
                    duration: Some(3),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    pump(&a, &b, &world).await;
    let messages = got.lock().unwrap().clone();
    assert_eq!(messages.len(), 3);
    let shape = |message: &MessageReceived| match message.media.as_ref().unwrap() {
        enums::DecryptedMessageMedia::Document(d) => (
            d.mime_type.clone(),
            d.size,
            d.attributes
                .iter()
                .map(Serializable::to_bytes)
                .collect::<Vec<_>>(),
        ),
        _ => panic!("Expected document"),
    };
    assert_eq!(shape(&messages[0]), shape(&messages[1]));
    assert_eq!(shape(&messages[1]), shape(&messages[2]));
    for (index, message) in messages.iter().enumerate() {
        let destination = directory.path().join(format!("out{index}"));
        b.save_message_file(message, &destination).await.unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), content);
    }
}
#[tokio::test]
async fn stream_starts_at_current_position() {
    // Rust Read may legally return fewer bytes than requested. Previewing must
    // still inspect the complete bounded header and restore the caller cursor.
    struct PartialRead(Cursor<Vec<u8>>);
    impl Read for PartialRead {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let maximum = output.len().min(3);
            self.0.read(&mut output[..maximum])
        }
    }
    impl Seek for PartialRead {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(position)
        }
    }
    let mut input = Cursor::new(b"HEADERbounded header from a partial reader".to_vec());
    input.set_position(6);
    let mut preview = FileSource::stream(Box::new(PartialRead(input)), "voice.ogg")
        .unwrap()
        .open()
        .unwrap();
    assert_eq!(
        preview.peek().unwrap(),
        b"bounded header from a partial reader"
    );
    assert_eq!(preview.reader().stream_position().unwrap(), 6);
    let mut rest = Vec::new();
    preview.reader().read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"bounded header from a partial reader");
    preview.rewind().unwrap();
    assert_eq!(preview.reader().stream_position().unwrap(), 6);
    let mut empty = FileSource::bytes(Vec::new(), "empty.ogg")
        .unwrap()
        .open()
        .unwrap();
    assert!(empty.peek().unwrap().is_empty());
    assert_eq!(empty.reader().stream_position().unwrap(), 0);
    let mut large = FileSource::bytes(
        vec![7; 2 * kiaro_secret_chat::ogg::HEADER_BYTES],
        "large.ogg",
    )
    .unwrap()
    .open()
    .unwrap();
    assert_eq!(
        large.peek().unwrap().len(),
        kiaro_secret_chat::ogg::HEADER_BYTES
    );
    assert_eq!(large.reader().stream_position().unwrap(), 0);
    assert_eq!(
        large.size,
        (2 * kiaro_secret_chat::ogg::HEADER_BYTES) as u64
    );
    struct BrokenRead {
        cursor: Cursor<Vec<u8>>,
        seen: bool,
    }
    impl Read for BrokenRead {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.seen {
                return Err(std::io::Error::other("synthetic private read failure"));
            }
            self.seen = true;
            let maximum = output.len().min(3);
            self.cursor.read(&mut output[..maximum])
        }
    }
    impl Seek for BrokenRead {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(position)
        }
    }
    let mut cursor = Cursor::new(b"HEADERbounded failing read".to_vec());
    cursor.set_position(6);
    let mut broken = FileSource::stream(
        Box::new(BrokenRead {
            cursor,
            seen: false,
        }),
        "broken.ogg",
    )
    .unwrap()
    .open()
    .unwrap();
    assert!(broken.peek().is_err());
    assert_eq!(
        broken.reader().stream_position().unwrap(),
        6,
        "failed preview must restore the caller cursor too"
    );

    let (a, b, world, id) = established().await;
    let got = collected(&b);
    let mut source = Cursor::new([b"HEADER".as_slice(), b"body".repeat(100).as_slice()].concat());
    source.set_position(6);
    a.send_file(
        id,
        FileSource::stream(Box::new(source), "notes.txt").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("body");
    let message = got.lock().unwrap()[0].clone();
    b.save_message_file(&message, &path).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"body".repeat(100));
}
#[tokio::test]
async fn bytes_source_needs_filename() {
    let (_, _, world, _) = established().await;
    assert!(FileSource::bytes(b"abc".to_vec(), "").is_err());
    assert_eq!(world.lock().unwrap().uploads, 0);
}
#[tokio::test]
async fn stream_source_needs_filename() {
    let (_, _, world, _) = established().await;
    assert!(FileSource::stream(Box::new(Cursor::new(b"abc".to_vec())), "").is_err());
    assert_eq!(world.lock().unwrap().uploads, 0);
}
struct Unseekable;
impl Read for Unseekable {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}
impl Seek for Unseekable {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Not seekable",
        ))
    }
}
#[tokio::test]
async fn unseekable_source_is_refused() {
    let (a, _, world, id) = established().await;
    assert!(
        a.send_file(
            id,
            FileSource::stream(Box::new(Unseekable), "x.bin").unwrap(),
            SendFileOptions::default()
        )
        .await
        .is_err()
    );
    assert_eq!(world.lock().unwrap().uploads, 0);
}
#[test]
fn encrypting_reader_needs_no_temporary_file() {
    let key = FileKey::generate().unwrap();
    let mut reader =
        media::EncryptingReader::new(Cursor::new(vec![42; 65537]), 65537, key.clone()).unwrap();
    let mut encrypted = Vec::new();
    reader.read_to_end(&mut encrypted).unwrap();
    let mut decoded = Vec::new();
    decrypt_file(&mut encrypted.as_slice(), &mut decoded, 65537, &key).unwrap();
    assert_eq!(decoded, vec![42; 65537]);
}
