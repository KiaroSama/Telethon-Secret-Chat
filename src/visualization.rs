//! The official client's 12 by 12 key picture, in least-significant-pair order.
use crate::crypto::SharedKey;
use crate::error::{Error, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
mod renderer;
pub use renderer::{KeyVisualization, PALETTE};
pub fn key_hash(key: &SharedKey) -> [u8; 36] {
    let a = Sha1::digest(key.expose());
    let b = Sha256::digest(key.expose());
    let mut hash = [0; 36];
    hash[..16].copy_from_slice(&a[..16]);
    hash[16..].copy_from_slice(&b[..20]);
    hash
}
pub fn key_visualization(hash: &[u8]) -> Result<KeyVisualization> {
    renderer::render(hash).ok_or(Error::KeyLength)
}
