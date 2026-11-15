//! MTProto 2.0 framing with grammers' public AES-IGE primitive.
use crate::error::{Error, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const KEY_LENGTH: usize = 256;
pub const HEADER_LENGTH: usize = 24;
pub const MIN_PADDING: usize = 12;
pub const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

/// A shared key deliberately has no Debug or Display implementation.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SharedKey([u8; KEY_LENGTH]);
impl SharedKey {
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes.try_into().map_err(|_| Error::KeyLength)?))
    }
    pub fn expose(&self) -> &[u8; KEY_LENGTH] {
        &self.0
    }
    pub fn fingerprint(&self) -> i64 {
        key_fingerprint(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Initiator,
    Responder,
}
impl Direction {
    pub fn offset(self) -> usize {
        match self {
            Self::Initiator => 0,
            Self::Responder => 8,
        }
    }
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
    Ok(bytes)
}
pub fn random_id() -> Result<i64> {
    Ok(i64::from_le_bytes(random_bytes()?))
}

pub fn key_fingerprint(shared_key: &[u8; KEY_LENGTH]) -> i64 {
    let digest = Sha1::digest(shared_key);
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[12..20]);
    i64::from_le_bytes(bytes)
}
pub fn compute_msg_key(shared_key: &SharedKey, payload: &[u8], direction: Direction) -> [u8; 16] {
    let x = direction.offset();
    let mut hash = Sha256::new();
    hash.update(&shared_key.0[88 + x..120 + x]);
    hash.update(payload);
    let digest = hash.finalize();
    let mut result = [0; 16];
    result.copy_from_slice(&digest[8..24]);
    result
}
pub fn derive_keys(
    shared_key: &SharedKey,
    msg_key: &[u8; 16],
    direction: Direction,
) -> ([u8; 32], [u8; 32]) {
    let x = direction.offset();
    let mut a = Sha256::new();
    a.update(msg_key);
    a.update(&shared_key.0[x..x + 36]);
    let a = a.finalize();
    let mut b = Sha256::new();
    b.update(&shared_key.0[x + 40..x + 76]);
    b.update(msg_key);
    let b = b.finalize();
    let mut key = [0; 32];
    let mut iv = [0; 32];
    key[..8].copy_from_slice(&a[..8]);
    key[8..24].copy_from_slice(&b[8..24]);
    key[24..].copy_from_slice(&a[24..]);
    iv[..8].copy_from_slice(&b[..8]);
    iv[8..24].copy_from_slice(&a[8..24]);
    iv[24..].copy_from_slice(&b[24..]);
    (key, iv)
}
pub fn pad_payload(payload: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if payload.len() > MAX_FRAME_LENGTH - HEADER_LENGTH - 32 {
        return Err(Error::FrameRejected);
    }
    let padding = MIN_PADDING + (16 - (payload.len() + MIN_PADDING) % 16) % 16;
    let mut result = Zeroizing::new(payload.to_vec());
    let start = result.len();
    result.resize(start + padding, 0);
    getrandom::fill(&mut result[start..]).map_err(|_| Error::Randomness)?;
    Ok(result)
}
pub fn encrypt_frame(shared_key: &SharedKey, body: &[u8], direction: Direction) -> Result<Vec<u8>> {
    let len = u32::try_from(body.len()).map_err(|_| Error::FrameRejected)?;
    let mut unpadded = Zeroizing::new(Vec::with_capacity(body.len() + 4));
    unpadded.extend_from_slice(&len.to_le_bytes());
    unpadded.extend_from_slice(body);
    let mut padded = pad_payload(&unpadded)?;
    let msg_key = compute_msg_key(shared_key, &padded, direction);
    let (key, iv) = derive_keys(shared_key, &msg_key, direction);
    let key = Zeroizing::new(key);
    let iv = Zeroizing::new(iv);
    grammers_crypto::aes::ige_encrypt(&mut padded, &key, &iv);
    let mut frame = Vec::with_capacity(HEADER_LENGTH + padded.len());
    frame.extend_from_slice(&shared_key.fingerprint().to_le_bytes());
    frame.extend_from_slice(&msg_key);
    frame.extend_from_slice(&padded);
    Ok(frame)
}
pub fn decrypt_frame(
    shared_key: &SharedKey,
    frame: &[u8],
    direction: Direction,
) -> Result<Zeroizing<Vec<u8>>> {
    if frame.len() < HEADER_LENGTH + 16
        || frame.len() > MAX_FRAME_LENGTH
        || !(frame.len() - HEADER_LENGTH).is_multiple_of(16)
    {
        return Err(Error::FrameRejected);
    }
    if frame[..8] == [0; 8] {
        return Err(Error::FrameRejected);
    }
    if !bool::from(frame[..8].ct_eq(&shared_key.fingerprint().to_le_bytes())) {
        return Err(Error::FrameRejected);
    }
    let msg_key: [u8; 16] = frame[8..24].try_into().map_err(|_| Error::FrameRejected)?;
    let (key, iv) = derive_keys(shared_key, &msg_key, direction);
    let key = Zeroizing::new(key);
    let iv = Zeroizing::new(iv);
    let mut plaintext = Zeroizing::new(frame[24..].to_vec());
    grammers_crypto::aes::ige_decrypt(&mut plaintext, &key, &iv);
    if !bool::from(msg_key.ct_eq(&compute_msg_key(shared_key, &plaintext, direction))) {
        return Err(Error::FrameRejected);
    }
    let declared = u32::from_le_bytes(
        plaintext[..4]
            .try_into()
            .map_err(|_| Error::FrameRejected)?,
    ) as usize;
    if declared > plaintext.len() - 4 || plaintext.len() - 4 - declared < MIN_PADDING {
        return Err(Error::FrameRejected);
    }
    // A receive-side 1024-byte padding ceiling rejects valid TDLib frames.
    let body = Zeroizing::new(plaintext[4..4 + declared].to_vec());
    Ok(body)
}

impl serde::Serialize for SharedKey {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let value = Zeroizing::new(hex::encode(self.0));
        serializer.serialize_str(&value)
    }
}
impl<'de> serde::Deserialize<'de> for SharedKey {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Zeroizing::new(<String as serde::Deserialize>::deserialize(deserializer)?);
        let mut bytes = Zeroizing::new([0u8; 256]);
        hex::decode_to_slice(&*value, &mut bytes[..])
            .map_err(|_| serde::de::Error::custom("Invalid shared key"))?;
        Ok(Self(*bytes))
    }
}
