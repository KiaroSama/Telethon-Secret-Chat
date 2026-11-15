#![allow(clippy::unwrap_used, clippy::expect_used)]
use kiaro_secret_chat::{
    Serializable,
    crypto::{self, Direction, SharedKey},
    enums,
    files::{self, FileKey},
    framing, types, visualization, wire,
};
use proptest::prelude::*;
fn key() -> SharedKey {
    SharedKey::from_slice(&(0..=255).collect::<Vec<u8>>()).unwrap()
}
fn message() -> enums::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1,
        ttl: 0,
        message: "Rust ✓ فارسی 日本語".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}
#[test]
fn shared_keys_require_fixed_width() {
    for len in [0, 1, 32, 255, 257, 1024] {
        assert!(SharedKey::from_slice(&vec![0; len]).is_err());
    }
    assert!(SharedKey::from_slice(&[0; 256]).is_ok());
}
#[test]
fn kdf_matches_independent_grammers_client_vector() {
    let (aes, iv) = crypto::derive_keys(
        &key(),
        &std::array::from_fn(|i| i as u8),
        Direction::Initiator,
    );
    assert_eq!(
        aes,
        [
            112, 78, 208, 156, 139, 65, 102, 138, 232, 249, 157, 36, 71, 56, 247, 29, 189, 220, 68,
            70, 155, 107, 189, 74, 168, 87, 61, 208, 66, 189, 5, 158
        ]
    );
    assert_eq!(
        iv,
        [
            77, 38, 96, 0, 165, 80, 237, 171, 191, 76, 124, 228, 15, 208, 4, 60, 201, 34, 48, 24,
            76, 211, 23, 165, 204, 156, 36, 130, 253, 59, 147, 24
        ]
    );
}
#[test]
fn kdf_matches_independent_grammers_server_vector() {
    let (aes, iv) = crypto::derive_keys(
        &key(),
        &std::array::from_fn(|i| i as u8),
        Direction::Responder,
    );
    assert_eq!(
        aes,
        [
            33, 119, 37, 121, 155, 36, 88, 6, 69, 129, 116, 161, 252, 251, 200, 131, 144, 104, 7,
            177, 80, 51, 253, 208, 234, 43, 77, 105, 207, 156, 54, 78
        ]
    );
    assert_eq!(
        iv,
        [
            102, 154, 101, 56, 145, 122, 79, 165, 108, 163, 35, 96, 164, 49, 201, 22, 11, 228, 173,
            136, 113, 64, 152, 13, 171, 145, 206, 123, 220, 71, 255, 188
        ]
    );
}
#[test]
fn mtproto2_frame_roundtrip_both_directions() {
    for direction in [Direction::Initiator, Direction::Responder] {
        let frame = crypto::encrypt_frame(&key(), b"hello", direction).unwrap();
        assert_eq!(
            &**crypto::decrypt_frame(&key(), &frame, direction)
                .as_ref()
                .unwrap(),
            b"hello"
        );
    }
}
#[test]
fn opposite_direction_is_rejected() {
    let frame = crypto::encrypt_frame(&key(), b"payload", Direction::Initiator).unwrap();
    assert!(crypto::decrypt_frame(&key(), &frame, Direction::Responder).is_err());
}
#[test]
fn every_single_byte_frame_change_is_rejected() {
    let frame = crypto::encrypt_frame(&key(), b"payload", Direction::Initiator).unwrap();
    for i in 0..frame.len() {
        let mut corrupt = frame.clone();
        corrupt[i] ^= 1;
        assert!(crypto::decrypt_frame(&key(), &corrupt, Direction::Initiator).is_err());
    }
}
#[test]
fn all_truncations_are_rejected() {
    let frame = crypto::encrypt_frame(&key(), b"payload", Direction::Initiator).unwrap();
    for end in 0..frame.len() {
        assert!(crypto::decrypt_frame(&key(), &frame[..end], Direction::Initiator).is_err());
    }
}
#[test]
fn cipher_padding_obeys_minimum_and_alignment() {
    for len in 0..1024 {
        let padded = crypto::pad_payload(&vec![0; len]).unwrap();
        assert_eq!(padded.len() % 16, 0);
        assert!((12..=27).contains(&(padded.len() - len)));
    }
}
#[test]
fn frame_padding_is_randomized() {
    let a = crypto::encrypt_frame(&key(), b"same", Direction::Initiator).unwrap();
    let b = crypto::encrypt_frame(&key(), b"same", Direction::Initiator).unwrap();
    assert_ne!(a, b);
}
#[test]
fn wrapper_rejects_trailing_bytes() {
    let mut bytes = framing::wrap(message(), 144, 0, 1).unwrap().to_bytes();
    assert!(framing::unwrap(&bytes).is_ok());
    bytes.push(0);
    assert!(framing::unwrap(&bytes).is_err());
}
#[test]
fn wrapper_refuses_wrong_family() {
    let bytes = enums::DecryptedMessageAction::Noop.to_bytes();
    assert!(framing::unwrap(&bytes).is_err());
}
#[test]
fn nested_constructor_must_match_result_family() {
    let mut bytes = framing::wrap(message(), 144, 0, 1).unwrap().to_bytes();
    bytes[48..52].copy_from_slice(&enums::DecryptedMessageAction::Noop.to_bytes());
    assert!(framing::unwrap(&bytes).is_err());
}
#[test]
fn invalid_utf8_is_refused_without_replacement() {
    let msg = types::DecryptedMessage {
        message: "x".into(),
        ..match message() {
            enums::DecryptedMessage::Message(m) => m,
            _ => unreachable!(),
        }
    };
    let mut bytes: Vec<u8> = enums::DecryptedMessage::from(msg).to_bytes();
    let p = bytes.iter().position(|b| *b == b'x').unwrap();
    bytes[p] = 255;
    assert!(wire::validate(&bytes, "DecryptedMessage").is_err());
}
#[test]
fn sequence_transform_has_mirrored_parity() {
    for n in [0, 1, 100, 1000] {
        assert_eq!(framing::transform_in_seq_no(n, true).unwrap(), 2 * n);
        assert_eq!(framing::transform_out_seq_no(n, true).unwrap(), 2 * n + 1);
        assert_eq!(framing::transform_in_seq_no(n, false).unwrap(), 2 * n + 1);
        assert_eq!(framing::transform_out_seq_no(n, false).unwrap(), 2 * n);
    }
}
#[test]
fn sequence_overflow_is_refused() {
    assert!(framing::transform_out_seq_no(i32::MAX, true).is_err());
    assert!(framing::transform_in_seq_no(-1, false).is_err());
}
#[test]
fn outgoing_layer_is_bounded() {
    assert_eq!(framing::outgoing_layer(46), 73);
    assert_eq!(framing::outgoing_layer(101), 101);
    assert_eq!(framing::outgoing_layer(1000), 144);
}
#[test]
fn capability_layer_never_decreases() {
    assert_eq!(framing::raise_remote_layer(144, 73), 144);
}
#[test]
fn file_fingerprint_independent_digest() {
    let key = FileKey::new(&[0; 32], &[0; 32]).unwrap();
    assert_eq!(key.fingerprint(), -1593105637);
}
#[test]
fn file_keys_are_fresh() {
    let a = FileKey::generate().unwrap();
    let b = FileKey::generate().unwrap();
    assert_ne!(a.key(), b.key());
    assert_ne!(a.iv(), b.iv());
}
#[test]
fn file_roundtrip_crosses_stream_chunk_boundary() {
    for size in [0, 1, 15, 16, 17, 65535, 65536, 65537, 131073] {
        let plain = vec![42; size];
        let key = FileKey::generate().unwrap();
        let mut encrypted = Vec::new();
        files::encrypt_file(&mut plain.as_slice(), &mut encrypted, size as u64, &key).unwrap();
        assert_eq!(
            encrypted.len() as u64,
            files::encrypted_size(size as u64).unwrap()
        );
        let mut decoded = Vec::new();
        files::decrypt_file(&mut encrypted.as_slice(), &mut decoded, size as u64, &key).unwrap();
        assert_eq!(decoded, plain);
    }
}
#[test]
fn truncated_file_preserves_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"existing").unwrap();
    let key = FileKey::generate().unwrap();
    assert!(files::save_file(&mut &b"short"[..], &path, 20, &key, key.fingerprint()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn wrong_file_fingerprint_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    let key = FileKey::generate().unwrap();
    assert!(files::save_file(&mut &b""[..], &path, 0, &key, key.fingerprint() ^ 1).is_err());
    assert!(!path.exists());
}
#[test]
fn extended_file_is_refused() {
    let key = FileKey::generate().unwrap();
    let mut encrypted = vec![];
    files::encrypt_file(&mut &b"data"[..], &mut encrypted, 4, &key).unwrap();
    encrypted.extend([0; 16]);
    assert!(files::decrypt_file(&mut encrypted.as_slice(), &mut vec![], 4, &key).is_err());
}
#[test]
fn changing_source_is_refused() {
    let key = FileKey::generate().unwrap();
    assert!(files::encrypt_file(&mut &b"more"[..], &mut vec![], 3, &key).is_err());
}
#[test]
fn key_visualization_little_endian_pixel_order() {
    let mut hash = [0; 36];
    hash[0] = 0b11100100;
    let picture = visualization::key_visualization(&hash).unwrap();
    assert_eq!(&picture.rows[0][..4], &[0, 1, 2, 3]);
    assert_eq!(picture.hex.len(), 64);
}
#[test]
fn key_visualization_requires_36_bytes() {
    for n in [0, 16, 32, 35, 37] {
        assert!(visualization::key_visualization(&vec![0; n]).is_err());
    }
}
#[test]
fn rekey_collision_uses_signed_long() {
    use kiaro_secret_chat::rekey::{Collision, resolve_collision};
    assert_eq!(resolve_collision(-1, 1), Collision::JoinTheirs);
    assert_eq!(resolve_collision(1, -1), Collision::AbandonTheirs);
    assert_eq!(resolve_collision(1, 1), Collision::AbortBoth);
}
#[test]
fn closed_chat_scrubs_all_keys() {
    use kiaro_secret_chat::chat::{Chat, ChatState};
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    chat.adopt_key(key());
    chat.previous_key = Some(key());
    chat.close();
    assert!(chat.key.is_none());
    assert!(chat.previous_key.is_none());
    assert_eq!(chat.state, ChatState::Closed);
    chat.validate().unwrap();
}
#[test]
fn official_client_frames_replay() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "vectors/fixtures/official-client-2026-09-29.json"
    ))
    .unwrap();
    let key =
        SharedKey::from_slice(&hex::decode(fixture["key"].as_str().unwrap()).unwrap()).unwrap();
    let mut chat = kiaro_secret_chat::chat::Chat::new(
        1,
        0,
        1,
        true,
        kiaro_secret_chat::chat::ChatState::Ready,
    );
    chat.out_seq_no = fixture["local_out_seq_no"].as_i64().unwrap() as i32;
    for entry in fixture["frames"].as_array().unwrap() {
        let frame = hex::decode(entry["frame"].as_str().unwrap()).unwrap();
        let body = crypto::decrypt_frame(&key, &frame, Direction::Responder).unwrap();
        let wrapper = framing::unwrap(&body).unwrap();
        let expected = &entry["expected"];
        assert_eq!(wrapper.layer as i64, expected["layer"].as_i64().unwrap());
        assert_eq!(
            wrapper.in_seq_no as i64,
            expected["in_seq_no"].as_i64().unwrap()
        );
        assert_eq!(
            wrapper.out_seq_no as i64,
            expected["out_seq_no"].as_i64().unwrap()
        );
        kiaro_secret_chat::sequence::accept(
            &mut chat,
            kiaro_secret_chat::chat::Envelope {
                body: body.to_vec(),
                file: None,
                date: 0,
            },
        )
        .unwrap();
    }
    assert_eq!(
        chat.in_seq_no as usize,
        fixture["frames"].as_array().unwrap().len()
    );
    assert!(chat.gaps.is_empty());
}
proptest! {
    #[test] fn arbitrary_bodies_roundtrip(body in proptest::collection::vec(any::<u8>(),0..8192),responder in any::<bool>()){
        let direction=if responder{Direction::Responder}else{Direction::Initiator};let frame=crypto::encrypt_frame(&key(),&body,direction).unwrap();prop_assert_eq!(&*crypto::decrypt_frame(&key(),&frame,direction).unwrap(),&body);
    }
    #[test] fn arbitrary_wire_input_never_panics(bytes in proptest::collection::vec(any::<u8>(),0..4096)){let _=framing::unwrap(&bytes);}
}
