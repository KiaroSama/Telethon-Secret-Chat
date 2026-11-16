//! Independent Telethon-produced MTProto 2.0 frames, including authenticated
//! malformed lengths and padding. Fixture material is public and synthetic.
#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto::{Direction, SharedKey, decrypt_frame};
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/receive-oracle.json")).unwrap()
}
fn key() -> SharedKey {
    SharedKey::from_slice(&hex::decode(fixture()["key"].as_str().unwrap()).unwrap()).unwrap()
}
fn frame(name: &str) -> Vec<u8> {
    hex::decode(fixture()["frames"][name].as_str().unwrap()).unwrap()
}
fn body() -> Vec<u8> {
    hex::decode(fixture()["body"].as_str().unwrap()).unwrap()
}
#[test]
fn independently_encrypted_frame_decrypts() {
    assert_eq!(
        decrypt_frame(&key(), &frame("valid_0"), Direction::Initiator)
            .unwrap()
            .as_slice(),
        body()
    );
}
#[test]
fn both_independent_directions_decrypt() {
    for (name, direction) in [
        ("valid_0", Direction::Initiator),
        ("valid_8", Direction::Responder),
    ] {
        assert_eq!(
            decrypt_frame(&key(), &frame(name), direction)
                .unwrap()
                .as_slice(),
            body()
        );
    }
}
#[test]
fn mismatched_fingerprint_refused() {
    let mut bytes = frame("valid_0");
    bytes[0] ^= 255;
    assert!(decrypt_frame(&key(), &bytes, Direction::Initiator).is_err());
}
#[test]
fn frame_encrypted_under_another_key_refused() {
    assert!(decrypt_frame(&key(), &frame("other_key"), Direction::Initiator).is_err());
}
#[test]
fn tampered_message_key_refused() {
    let mut bytes = frame("valid_0");
    bytes[8] ^= 1;
    assert!(decrypt_frame(&key(), &bytes, Direction::Initiator).is_err());
}
#[test]
fn tampered_ciphertext_refused() {
    let mut bytes = frame("valid_0");
    let end = bytes.len() - 1;
    bytes[end] ^= 1;
    assert!(decrypt_frame(&key(), &bytes, Direction::Initiator).is_err());
}
#[test]
fn wrong_direction_refused() {
    assert!(decrypt_frame(&key(), &frame("valid_0"), Direction::Responder).is_err());
}
#[test]
fn authenticated_length_beyond_buffer_refused() {
    assert!(decrypt_frame(&key(), &frame("beyond"), Direction::Initiator).is_err());
}
#[test]
fn authenticated_off_by_four_length_refused() {
    assert!(decrypt_frame(&key(), &frame("off_by_four"), Direction::Initiator).is_err());
}
#[test]
fn authenticated_negative_length_refused() {
    assert!(decrypt_frame(&key(), &frame("negative"), Direction::Initiator).is_err());
}
#[test]
fn authenticated_eleven_padding_bytes_refused() {
    assert!(decrypt_frame(&key(), &frame("short_padding"), Direction::Initiator).is_err());
}
#[test]
fn unaligned_payload_frame_refused() {
    assert!(decrypt_frame(&key(), &frame("unaligned_payload"), Direction::Initiator).is_err());
    let mut bytes = frame("valid_0");
    bytes.push(0);
    assert!(decrypt_frame(&key(), &bytes, Direction::Initiator).is_err());
}
#[test]
fn receiver_accepts_reference_padding_beyond_sender_guideline() {
    assert_eq!(
        decrypt_frame(&key(), &frame("long_padding"), Direction::Initiator)
            .unwrap()
            .as_slice(),
        body()
    );
}
macro_rules! truncated {
    ($name:ident,$size:expr) => {
        #[test]
        fn $name() {
            assert!(
                decrypt_frame(&key(), &frame("valid_0")[..$size], Direction::Initiator).is_err()
            );
        }
    };
}
truncated!(truncated_header_0, 0);
truncated!(truncated_header_8, 8);
truncated!(truncated_header_23, 23);
truncated!(truncated_header_24, 24);
truncated!(truncated_header_25, 25);
#[test]
fn refusal_contains_no_key_plaintext_or_ciphertext() {
    let mut bytes = frame("valid_0");
    let end = bytes.len() - 1;
    bytes[end] ^= 1;
    let error = decrypt_frame(&key(), &bytes, Direction::Initiator).unwrap_err();
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains(std::str::from_utf8(&body()).unwrap()));
    assert!(
        !rendered
            .to_lowercase()
            .contains(&hex::encode(key().expose())[..32])
    );
    assert!(!rendered.to_lowercase().contains(&hex::encode(bytes)[..32]));
}
