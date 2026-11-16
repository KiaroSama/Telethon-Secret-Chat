#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    files::{FileKey, decrypt_file},
    media::EncryptingReader,
};
use std::io::{Cursor, Read};
fn check(size: usize, piece: usize) {
    let key = FileKey::generate().unwrap();
    let mut data = vec![0; size];
    getrandom::fill(&mut data).unwrap();
    let mut padding = vec![0; (16 - size % 16) % 16];
    getrandom::fill(&mut padding).unwrap();
    let mut reader = EncryptingReader::new_with_padding(
        Cursor::new(data.clone()),
        size as u64,
        key.clone(),
        padding.clone(),
    )
    .unwrap();
    assert_eq!(reader.size, size as u64 + padding.len() as u64);
    let mut streamed = Vec::new();
    let mut buffer = vec![0; piece];
    loop {
        let read = reader.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        streamed.extend_from_slice(&buffer[..read]);
    }
    let mut one_shot = [data.clone(), padding].concat();
    grammers_crypto::aes::ige_encrypt(&mut one_shot, key.key(), key.iv());
    assert_eq!(
        streamed, one_shot,
        "streamed IGE must carry IV across upload chunks"
    );
    struct Pieces<'a>(&'a [u8]);
    impl Read for Pieces<'_> {
        fn read(&mut self, to: &mut [u8]) -> std::io::Result<usize> {
            let len = 1000.min(self.0.len()).min(to.len());
            to[..len].copy_from_slice(&self.0[..len]);
            self.0 = &self.0[len..];
            Ok(len)
        }
    }
    let mut result = Vec::new();
    decrypt_file(&mut Pieces(&streamed), &mut result, size as u64, &key).unwrap();
    assert_eq!(result, data);
}
#[test]
fn stream_1024_0() {
    check(0, 1024)
}
#[test]
fn stream_1024_1() {
    check(1, 1024)
}
#[test]
fn stream_1024_15() {
    check(15, 1024)
}
#[test]
fn stream_1024_16() {
    check(16, 1024)
}
#[test]
fn stream_1024_17() {
    check(17, 1024)
}
#[test]
fn stream_1024_1023() {
    check(1023, 1024)
}
#[test]
fn stream_1024_1024() {
    check(1024, 1024)
}
#[test]
fn stream_1024_1025() {
    check(1025, 1024)
}
#[test]
fn stream_1024_150007() {
    check(150007, 1024)
}
#[test]
fn stream_65536_0() {
    check(0, 65536)
}
#[test]
fn stream_65536_1() {
    check(1, 65536)
}
#[test]
fn stream_65536_15() {
    check(15, 65536)
}
#[test]
fn stream_65536_16() {
    check(16, 65536)
}
#[test]
fn stream_65536_17() {
    check(17, 65536)
}
#[test]
fn stream_65536_1023() {
    check(1023, 65536)
}
#[test]
fn stream_65536_1024() {
    check(1024, 65536)
}
#[test]
fn stream_65536_1025() {
    check(1025, 65536)
}
#[test]
fn stream_65536_150007() {
    check(150007, 65536)
}
