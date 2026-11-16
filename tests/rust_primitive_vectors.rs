#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    chat::{Chat, Envelope},
    crypto::{self, SharedKey},
    enums, framing, sequence,
};
#[test]
fn official_client_frames_replay_offline() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/fixtures");
    let mut paths = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("official-client-")
                && p.extension().is_some_and(|e| e == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "the archived independent official-client fixture must remain available"
    );
    for path in paths {
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(document["provenance"].as_str().unwrap().contains("closed"));
        let key = SharedKey::from_slice(&hex::decode(document["key"].as_str().unwrap()).unwrap())
            .unwrap();
        let mut chat = Chat::for_role(1, 0, 1, document["is_outbound"].as_bool().unwrap());
        chat.adopt_key(key.clone()).unwrap();
        chat.out_seq_no = document["local_out_seq_no"]
            .as_i64()
            .unwrap()
            .try_into()
            .unwrap();
        let mut delivered = Vec::new();
        let frames = document["frames"].as_array().unwrap();
        for entry in frames {
            let frame = hex::decode(entry["frame"].as_str().unwrap()).unwrap();
            let body = crypto::decrypt_frame(&key, &frame, chat.in_direction()).unwrap();
            let wrapper = framing::unwrap(&body).unwrap();
            let expected = &entry["expected"];
            assert_eq!(
                wrapper.layer,
                i32::try_from(expected["layer"].as_i64().unwrap()).unwrap()
            );
            assert_eq!(
                wrapper.in_seq_no,
                i32::try_from(expected["in_seq_no"].as_i64().unwrap()).unwrap()
            );
            assert_eq!(
                wrapper.out_seq_no,
                i32::try_from(expected["out_seq_no"].as_i64().unwrap()).unwrap()
            );
            let (name, text) = match &wrapper.message {
                enums::DecryptedMessage::Message(m) => {
                    ("DecryptedMessage", Some(m.message.as_str()))
                }
                enums::DecryptedMessage::Service(_) => ("DecryptedMessageService", None),
                enums::DecryptedMessage::ServiceLegacyaa48327d(_) => {
                    ("DecryptedMessageService8", None)
                }
                _ => panic!("unrecognized captured shape"),
            };
            assert_eq!(name, expected["type"].as_str().unwrap());
            assert_eq!(text, expected["text"].as_str());
            let accepted = sequence::accept(
                &mut chat,
                Envelope {
                    body: body.to_vec(),
                    file: None,
                    date: 0,
                },
            )
            .unwrap();
            assert!(accepted.resend.is_none());
            delivered.extend(accepted.ready);
        }
        assert_eq!(delivered.len(), frames.len());
        assert_eq!(chat.in_seq_no as usize, frames.len());
        assert!(chat.gaps.is_empty());
        let needle = document["reply_contains"].as_str().unwrap();
        assert!(delivered.iter().any(|(_,e)|matches!(framing::unwrap(&e.body).unwrap().message,enums::DecryptedMessage::Message(m)if m.message.contains(needle))));
    }
}
#[test]
#[allow(unexpected_cfgs)]
fn ci_aes_backend_matches_declared_leg() {
    if let Ok(expected) = std::env::var("TSC_RUST_AES_BACKEND") {
        assert!(["soft", "default"].contains(&expected.as_str()));
        assert_eq!(
            cfg!(aes_backend = "soft"),
            expected == "soft",
            "CI declaration must match actual compile-time RustCrypto backend selection"
        );
    }
    let key: [u8; 32] =
        hex::decode("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4")
            .unwrap()
            .try_into()
            .unwrap();
    let input = hex::decode("6bc1bee22e409f96e93d7e117393172a").unwrap();
    let mut encrypted = input.clone();
    grammers_crypto::aes::ige_encrypt(&mut encrypted, &key, &[0; 32]);
    assert_eq!(
        hex::encode(&encrypted),
        "f3eed1bdb5d2a03c064b5a7e3db181f8",
        "NIST SP800-38A AES256 first block, zero IGE initial vectors"
    );
    grammers_crypto::aes::ige_decrypt(&mut encrypted, &key, &[0; 32]);
    assert_eq!(encrypted, input);
}
