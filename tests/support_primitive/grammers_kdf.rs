#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto::{self, Direction, SharedKey};
fn key() -> SharedKey {
    SharedKey::from_slice(&(0..=255).collect::<Vec<u8>>()).unwrap()
}
fn client() {
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
fn server() {
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
pub fn check(offset: u8) {
    if offset == 0 { client() } else { server() }
}
