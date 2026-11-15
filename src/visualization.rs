//! The official client's 12 by 12 key picture, in least-significant-pair order.
use crate::crypto::SharedKey;
use crate::error::{Error, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
pub const PALETTE: [&str; 4] = ["#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9"];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyVisualization {
    pub rows: [[u8; 12]; 12],
    pub hex: String,
}
pub fn key_hash(key: &SharedKey) -> [u8; 36] {
    let a = Sha1::digest(key.expose());
    let b = Sha256::digest(key.expose());
    let mut hash = [0; 36];
    hash[..16].copy_from_slice(&a[..16]);
    hash[16..].copy_from_slice(&b[..20]);
    hash
}
pub fn key_visualization(hash: &[u8]) -> Result<KeyVisualization> {
    if hash.len() != 36 {
        return Err(Error::KeyLength);
    }
    let mut rows = [[0; 12]; 12];
    for n in 0..144 {
        rows[n / 12][n % 12] = (hash[n / 4] >> (2 * (n % 4))) & 3;
    }
    Ok(KeyVisualization {
        rows,
        hex: hex::encode(&hash[..32]),
    })
}
