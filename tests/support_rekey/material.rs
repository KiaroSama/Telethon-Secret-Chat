#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto::SharedKey;
pub fn key(mult: u8, add: u8) -> SharedKey {
    SharedKey::from_slice(&std::array::from_fn::<_, 256, _>(|i| {
        (i as u8).wrapping_mul(mult).wrapping_add(add)
    }))
    .unwrap()
}
