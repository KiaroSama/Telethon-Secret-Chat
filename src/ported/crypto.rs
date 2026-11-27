//! The Rust port of `rust/adapters/crypto_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use aes::{
    Aes256,
    cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit},
};
use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const KEY_LENGTH: usize = 256;
const HEADER: usize = 24;
const BLOCK: usize = 16;
const MIN_PADDING: usize = 12;
const MAX_PADDING: usize = 1024;

fn direction(x: i64) -> Result<usize, String> {
    match x {
        0 => Ok(0),
        8 => Ok(8),
        _ => Err("ValueError: x must be 0 or 8: protocol-reference.md §2.5".into()),
    }
}

fn check_key(key: &[u8]) -> Result<(), String> {
    if key.len() != KEY_LENGTH {
        return Err(format!(
            "ValueError: a shared key is exactly {KEY_LENGTH} bytes: protocol-reference.md §1.4"
        ));
    }
    Ok(())
}

fn hash_pair(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(a);
    hash.update(b);
    hash.finalize().into()
}

fn key_slice(key: &[u8], start: usize, end: usize) -> &[u8] {
    &key[start.min(key.len())..end.min(key.len())]
}

fn fingerprint(key: &[u8]) -> [u8; 8] {
    let digest = Sha1::digest(key);
    let mut out = [0; 8];
    out.copy_from_slice(&digest[12..20]);
    out
}

fn message_key(key: &[u8], payload: &[u8], x: usize) -> [u8; 16] {
    let digest = hash_pair(key_slice(key, 88 + x, 120 + x), payload);
    let mut out = [0; 16];
    out.copy_from_slice(&digest[8..24]);
    out
}

fn derive(key: &[u8], message: &[u8], x: usize) -> ([u8; 32], [u8; 32]) {
    let a = hash_pair(message, key_slice(key, x, x + 36));
    let b = hash_pair(key_slice(key, 40 + x, 76 + x), message);
    let mut aes_key = a;
    let mut iv = b;
    aes_key[8..24].copy_from_slice(&b[8..24]);
    iv[8..24].copy_from_slice(&a[8..24]);
    (aes_key, iv)
}

fn pad(payload: &mut Vec<u8>) -> Result<(), String> {
    let extra = MIN_PADDING + (BLOCK - (payload.len() % BLOCK + MIN_PADDING) % BLOCK) % BLOCK;
    let start = payload.len();
    payload.resize(start + extra, 0);
    getrandom::fill(&mut payload[start..])
        .map_err(|_| "RuntimeError: OS randomness unavailable".into())
}

fn ige(payload: &mut [u8], key: [u8; 32], iv: [u8; 32], encrypt: bool) {
    let key = Zeroizing::new(key);
    let iv = Zeroizing::new(iv);
    let cipher = Aes256::new(&Array::from(*key));
    let (mut previous_cipher, mut previous_plain) = ([0; BLOCK], [0; BLOCK]);
    previous_cipher.copy_from_slice(&iv[..BLOCK]);
    previous_plain.copy_from_slice(&iv[BLOCK..]);
    for chunk in payload.as_chunks_mut::<BLOCK>().0 {
        let mut input = [0; BLOCK];
        input.copy_from_slice(chunk);
        let (before, after) = if encrypt {
            (&previous_cipher, &previous_plain)
        } else {
            (&previous_plain, &previous_cipher)
        };
        let mut block = Array::from(std::array::from_fn(|i| input[i] ^ before[i]));
        if encrypt {
            cipher.encrypt_block(&mut block);
        } else {
            cipher.decrypt_block(&mut block);
        }
        for i in 0..BLOCK {
            chunk[i] = block[i] ^ after[i];
        }
        if encrypt {
            previous_plain = input;
            previous_cipher.copy_from_slice(chunk);
        } else {
            previous_cipher = input;
            previous_plain.copy_from_slice(chunk);
        }
    }
}

fn encode(key: &[u8], body: &[u8], x: i64) -> Result<Vec<u8>, String> {
    let x = direction(x)?;
    check_key(key)?;
    if !body.len().is_multiple_of(4) {
        return Err("ValueError: serialized TL body length must be divisible by four".into());
    }
    let size = u32::try_from(body.len()).map_err(|_| "OverflowError: int too big to convert")?;
    let mut payload = size.to_le_bytes().to_vec();
    payload.extend_from_slice(body);
    pad(&mut payload)?;
    let message = message_key(key, &payload, x);
    let (aes_key, iv) = derive(key, &message, x);
    ige(&mut payload, aes_key, iv, true);
    let mut frame = fingerprint(key).to_vec();
    frame.extend_from_slice(&message);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode(key: &[u8], frame: &[u8], x: usize) -> Result<Vec<u8>, &'static str> {
    if frame.len() < HEADER + BLOCK {
        return Err("the frame is too short to hold a message");
    }
    if !bool::from(frame[..8].ct_eq(&fingerprint(key))) {
        return Err("the key fingerprint does not name a key this chat holds");
    }
    let mut payload = Zeroizing::new(frame[HEADER..].to_vec());
    if !payload.len().is_multiple_of(BLOCK) {
        return Err("the ciphertext is not a whole number of cipher blocks");
    }
    let message = &frame[8..HEADER];
    let (aes_key, iv) = derive(key, message, x);
    ige(&mut payload, aes_key, iv, false);
    if !bool::from(message.ct_eq(&message_key(key, &payload, x))) {
        return Err("the recomputed message key does not match the one received");
    }
    if !payload.len().is_multiple_of(BLOCK) {
        return Err("the decrypted length is not a multiple of 16");
    }
    let mut size = [0; 4];
    size.copy_from_slice(&payload[..4]);
    let size = u32::from_le_bytes(size) as usize;
    if !size.is_multiple_of(4) {
        return Err("the declared body length is not divisible by four");
    }
    let rest = &payload[4..];
    if size > rest.len() {
        return Err("the declared body length runs past the decrypted buffer");
    }
    if rest.len() - size < MIN_PADDING {
        return Err("the padding is shorter than the 12 bytes required");
    }
    if rest.len() - size > MAX_PADDING {
        return Err("the padding exceeds the maximum of 1024 bytes");
    }
    Ok(rest[..size].to_vec())
}

pub(crate) fn key_fingerprint_native(shared_key: &[u8]) -> Result<i64, String> {
    check_key(shared_key)?;
    Ok(i64::from_le_bytes(fingerprint(shared_key)))
}

pub(crate) fn compute_msg_key_native(
    shared_key: &[u8],
    payload: &[u8],
    x: i64,
) -> Result<[u8; 16], String> {
    check_key(shared_key)?;
    Ok(message_key(shared_key, payload, direction(x)?))
}

pub(crate) fn derive_keys_native(
    shared_key: &[u8],
    msg_key: &[u8],
    x: i64,
) -> Result<([u8; 32], [u8; 32]), String> {
    check_key(shared_key)?;
    if msg_key.len() != 16 {
        return Err("ValueError: a message key is exactly 16 bytes".into());
    }
    Ok(derive(shared_key, msg_key, direction(x)?))
}

pub(crate) fn decrypt_frame_native(
    shared_key: &[u8],
    frame: &[u8],
    x: i64,
) -> Result<Vec<u8>, String> {
    check_key(shared_key)?;
    decode(shared_key, frame, direction(x)?).map_err(String::from)
}

/// Port of `key_fingerprint` (`rust/adapters/crypto_adapter.py:11`).
pub fn key_fingerprint(shared_key: Vec<u8>) -> Result<i64, String> {
    let shared_key = Zeroizing::new(shared_key);
    key_fingerprint_native(&shared_key)
}

/// Port of `compute_msg_key` (`rust/adapters/crypto_adapter.py:15`).
pub fn compute_msg_key(shared_key: Vec<u8>, payload: Vec<u8>, x: i64) -> Result<Vec<u8>, String> {
    let shared_key = Zeroizing::new(shared_key);
    let payload = Zeroizing::new(payload);
    compute_msg_key_native(&shared_key, &payload, x).map(|key| key.to_vec())
}

/// Port of `derive_keys` (`rust/adapters/crypto_adapter.py:19`).
pub fn derive_keys(
    shared_key: Vec<u8>,
    msg_key: Vec<u8>,
    x: i64,
) -> Result<serde_json::Value, String> {
    let shared_key = Zeroizing::new(shared_key);
    let (key, iv) = derive_keys_native(&shared_key, &msg_key, x)?;
    Ok(json!({"$tuple": [{"$bytes": key}, {"$bytes": iv}]}))
}

/// Port of `pad_payload` (`rust/adapters/crypto_adapter.py:23`).
pub fn pad_payload(payload: Vec<u8>) -> Result<serde_json::Value, String> {
    let mut output = payload.clone();
    pad(&mut output)?;
    Ok(json!({"length": output.len(), "prefix_preserved": output.starts_with(&payload)}))
}

/// Port of `encrypt_frame` (`rust/adapters/crypto_adapter.py:28`).
pub fn encrypt_frame(
    shared_key: Vec<u8>,
    body: Vec<u8>,
    x: i64,
) -> Result<serde_json::Value, String> {
    let shared_key = Zeroizing::new(shared_key);
    let body = Zeroizing::new(body);
    let frame = encode(&shared_key, &body, x)?;
    let decoded = decode(&shared_key, &frame, direction(x)?).map_err(String::from)?;
    Ok(json!({"length": frame.len(), "body_preserved": decoded == *body}))
}

/// Port of `decrypt_frame` (`rust/adapters/crypto_adapter.py:34`).
///
/// `chat_id` defaults to `None` in the source.
pub fn decrypt_frame(
    shared_key: Vec<u8>,
    frame: Vec<u8>,
    x: i64,
    chat_id: Option<i64>,
) -> Result<serde_json::Value, String> {
    let shared_key = Zeroizing::new(shared_key);
    let x = direction(x)?;
    check_key(&shared_key)?;
    Ok(match decode(&shared_key, &frame, x) {
        Ok(body) => json!({"body": {"$bytes": body}}),
        Err(reason) => {
            let prefix = chat_id.map_or_else(String::new, |id| format!("[chat {id}] "));
            json!({"error_type": "MessageRejected", "reason": reason, "message": format!("{prefix}rejected a received message: {reason}"), "fatal": false})
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Telethon's MTProtoState._calc_key for the public inputs below.
    const EXPECTED_KEY: [u8; 32] = [
        112, 78, 208, 156, 139, 65, 102, 138, 232, 249, 157, 36, 71, 56, 247, 29, 189, 220, 68, 70,
        155, 107, 189, 74, 168, 87, 61, 208, 66, 189, 5, 158,
    ];

    #[test]
    fn preserves_padding_and_rejects_short_frame() -> Result<(), String> {
        let padded = pad_payload(vec![1, 2, 3])?;
        assert_eq!(padded["length"], 16);
        assert_eq!(padded["prefix_preserved"], true);
        let rejected = decrypt_frame(vec![0; 256], vec![0; 39], 0, Some(2))?;
        assert_eq!(rejected["error_type"], "MessageRejected");
        assert_eq!(
            rejected["reason"],
            "the frame is too short to hold a message"
        );
        Ok(())
    }

    #[test]
    fn decrypts_official_client_capture() -> Result<(), String> {
        fn unhex(text: Option<&str>) -> Result<Vec<u8>, String> {
            let input = text.ok_or("Missing fixture field")?;
            (0..input.len())
                .step_by(2)
                .map(|i| {
                    input
                        .get(i..i + 2)
                        .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                        .ok_or_else(|| "Malformed fixture hex".into())
                })
                .collect()
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/vectors/fixtures/official-client-2026-09-29.json"
        ))
        .map_err(|_| "Invalid official fixture")?;
        let key = unhex(fixture["key"].as_str())?;
        let x = if fixture["is_outbound"] == true { 8 } else { 0 };
        let frames = fixture["frames"]
            .as_array()
            .filter(|frames| !frames.is_empty())
            .ok_or("Missing official frames")?;
        for entry in frames {
            let frame = unhex(entry["frame"].as_str())?;
            let payload = decode(&key, &frame, x)?;
            let text = entry["expected"]["text"]
                .as_str()
                .filter(|text| !text.is_empty());
            assert!(
                text.is_none_or(|text| payload
                    .windows(text.len())
                    .any(|bytes| bytes == text.as_bytes())),
                "Official frame body differs"
            );
        }
        Ok(())
    }

    #[test]
    fn public_kdf_vector_agrees_with_telethon() -> Result<(), String> {
        let key = (0..=255u8).collect();
        let message_key = (0..16u8).collect();
        let keys = derive_keys(key, message_key, 0)?;
        let first = keys["$tuple"][0]["$bytes"]
            .as_array()
            .ok_or("missing AES key")?;
        let bytes: Option<Vec<u8>> = first
            .iter()
            .map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
            .collect();
        assert_eq!(bytes.as_deref(), Some(&EXPECTED_KEY[..]));
        Ok(())
    }
}
