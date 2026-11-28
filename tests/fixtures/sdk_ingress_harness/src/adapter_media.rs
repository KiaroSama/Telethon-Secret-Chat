use super::server::Server;
use kiaro_secret_chat::{
    files::{FileKey, decrypt_file, encrypt_file},
    grammers_tl_types::{self as tl, Identifiable},
    manager::Transport,
    media::{FileSource, MediaReference},
    transport::GrammersTransport,
    types,
};
use std::sync::{Arc, Mutex};
pub async fn run(transport: &GrammersTransport, model: &Arc<Mutex<Server>>) {
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    for size in [65, 11 * 1024 * 1024] {
        let plaintext = vec![0xa7; size];
        let mut input = FileSource::bytes(plaintext.clone(), "native-fixture.bin")
            .unwrap()
            .open()
            .unwrap();
        let file = Transport::upload(transport, &mut input, &key)
            .await
            .unwrap();
        let (id, parts, checksum) = match file {
            tl::enums::InputEncryptedFile::Uploaded(f) => {
                assert_eq!(size, 65);
                assert_eq!(f.key_fingerprint, key.fingerprint());
                (f.id, f.parts, Some(f.md5_checksum))
            }
            tl::enums::InputEncryptedFile::BigUploaded(f) => {
                assert!(size > 10 * 1024 * 1024);
                assert_eq!(f.key_fingerprint, key.fingerprint());
                (f.id, f.parts, None)
            }
            _ => unreachable!(),
        };
        let bytes = model.lock().unwrap().files[&id].clone();
        assert_eq!(parts as usize, bytes.len().div_ceil(512 * 1024));
        assert_eq!(bytes.len(), size.div_ceil(16) * 16);
        if let Some(checksum) = checksum {
            let expected = kiaro_secret_chat::media::md5_digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(checksum, expected);
        }
        let mut decoded = Vec::new();
        decrypt_file(&mut &bytes[..], &mut decoded, size as u64, &key).unwrap();
        assert_eq!(decoded, plaintext);
    }
    let plaintext = vec![0x5a; 65];
    let mut encrypted = Vec::new();
    encrypt_file(&mut &plaintext[..], &mut encrypted, 65, &key).unwrap();
    model.lock().unwrap().download = encrypted.clone();
    let reference = MediaReference::new(
        7,
        &tl::types::EncryptedFile {
            id: 9,
            access_hash: 99,
            size: 80,
            dc_id: 4,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: 65,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    let before = model.lock().unwrap().calls.len();
    assert!(
        Transport::download_bytes(transport, &reference, 64)
            .await
            .is_err()
    );
    assert_eq!(model.lock().unwrap().calls.len(), before);
    assert_eq!(
        Transport::download_bytes(transport, &reference, 65)
            .await
            .unwrap(),
        plaintext
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/native.bin");
    Transport::download(transport, &reference, &path)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), plaintext);
    assert!(
        model
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(_, id)| *id == tl::functions::upload::GetFile::CONSTRUCTOR_ID)
            .all(|(dc, _)| *dc == 4)
    );
    for length in [16, 96] {
        model.lock().unwrap().download = vec![0; length];
        assert!(
            Transport::download_bytes(transport, &reference, 65)
                .await
                .is_err()
        );
    }
    model.lock().unwrap().download = encrypted;
    model.lock().unwrap().failure = Some("FILE_REFERENCE_EXPIRED");
    assert!(
        Transport::download_bytes(transport, &reference, 65)
            .await
            .is_err()
    );
    model.lock().unwrap().failure = Some("FILE_REFERENCE_EXPIRED");
    assert!(
        Transport::download(transport, &reference, &path)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), plaintext);
}
