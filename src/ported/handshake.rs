//! The Rust port of `rust/adapters/handshake_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use crypto_bigint::{
    BoxedUint, Odd,
    modular::{BoxedMontyForm, BoxedMontyParams},
};
use num_bigint::{BigInt, Sign};
use serde_json::json;
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const SECRET_BITS: usize = 2048;

/// Native 2048-bit DH path: keep the exponent and shared result out of arbitrary-precision integers.
pub(crate) fn fixed_power(base: &[u8], secret: &[u8], modulus: &[u8]) -> Result<[u8; 256], String> {
    if modulus.len() != 256 || modulus[0] & 128 == 0 || base.len() > 256 {
        return Err("ValueError: DH parameters must fit the 2048-bit group".into());
    }
    if secret.is_empty()
        || secret.len() > 256
        || secret.iter().fold(0u8, |bits, &byte| bits | byte) == 0
    {
        return Err("ValueError: invalid DH secret".into());
    }
    let mut padded_base = [0u8; 256];
    padded_base[256 - base.len()..].copy_from_slice(base);
    if padded_base.as_slice() >= modulus {
        return Err("ValueError: DH base is outside the group".into());
    }
    let decode = |bytes: &[u8]| {
        BoxedUint::from_be_slice(bytes, SECRET_BITS as u32)
            .map_err(|_| "ValueError: invalid 2048-bit DH parameter".to_owned())
    };
    let exponent = Zeroizing::new(decode(secret)?);
    let odd = Option::<Odd<BoxedUint>>::from(Odd::new(decode(modulus)?))
        .ok_or("ValueError: 2048-bit modulus must be odd")?;
    let params = BoxedMontyParams::new(odd);
    let base = BoxedMontyForm::new(decode(&padded_base)?, &params);
    // pow uses the fixed bits_precision (2048), not the exponent's significant bit count.
    let powered = Zeroizing::new(base.pow(&exponent));
    let result = Zeroizing::new(powered.retrieve());
    let bytes = Zeroizing::new(result.to_be_bytes());
    let mut output = [0u8; 256];
    output.copy_from_slice(&bytes);
    Ok(output)
}

fn integer(raw: &[u8]) -> BigInt {
    BigInt::from_signed_bytes_be(raw)
}

fn pow(base: BigInt, exp: BigInt, modulus: BigInt) -> Result<BigInt, String> {
    let m = BigInt::from(modulus.magnitude().clone());
    if m == BigInt::ZERO {
        return Err("ValueError: pow() 3rd argument cannot be 0".into());
    }
    if m == BigInt::from(1u8) {
        return Ok(BigInt::ZERO);
    }
    let reduced = ((base % &m) + &m) % &m;
    let reduced = if exp.sign() == Sign::Minus {
        reduced
            .modinv(&m)
            .ok_or("ValueError: base is not invertible for the given modulus")?
    } else {
        reduced
    };
    let mut result = if m.bits() == SECRET_BITS as u64
        && exp.bits() <= SECRET_BITS as u64
        && exp.sign() != Sign::Minus
    {
        let decode = |n: &BigInt| {
            BoxedUint::from_be_slice(&n.magnitude().to_bytes_be(), SECRET_BITS as u32)
                .map_err(|_| "ValueError: invalid 2048-bit parameter")
        };
        let odd = Option::<Odd<BoxedUint>>::from(Odd::new(decode(&m)?))
            .ok_or("ValueError: 2048-bit modulus must be odd")?;
        BigInt::from_bytes_be(
            Sign::Plus,
            &decode(&reduced)?
                .pow_mod(&decode(&exp)?, &odd)
                .to_be_bytes(),
        )
    } else {
        reduced.modpow(&BigInt::from(exp.magnitude().clone()), &m)
    };
    if modulus.sign() == Sign::Minus && result != BigInt::ZERO {
        result -= m;
    }
    Ok(result)
}

fn refusal(reason: &str, chat_id: Option<i64>) -> serde_json::Value {
    let chat = chat_id.map_or_else(String::new, |id| format!("[chat {id}] "));
    json!({"error_type":"ParameterRejected","reason":reason,"message":format!("{chat}refused the key exchange: {reason}")})
}

/// Port of `generate_secret` (`rust/adapters/handshake_adapter.py:19`).
pub fn generate_secret() -> Result<serde_json::Value, String> {
    let mut entropy = [0u8; SECRET_BITS / 8];
    getrandom::fill(&mut entropy).map_err(|_| "RuntimeError: OS randomness unavailable")?;
    entropy[0] |= 128;
    let value = num_bigint::BigUint::from_bytes_be(&entropy);
    Ok(json!({"bits":value.bits(),"positive":value.bits() > 0}))
}

/// Port of `public_value` (`rust/adapters/handshake_adapter.py:24`).
pub fn public_value(g: Vec<u8>, secret: Vec<u8>, p: Vec<u8>) -> Result<String, String> {
    Ok(pow(integer(&g), integer(&secret), integer(&p))?.to_string())
}

/// Port of `shared_key` (`rust/adapters/handshake_adapter.py:28`).
///
/// `chat_id` defaults to `None` in the source.
pub fn shared_key(
    peer_value: Vec<u8>,
    secret: Vec<u8>,
    p: Vec<u8>,
    chat_id: Option<i64>,
) -> Result<serde_json::Value, String> {
    let (peer, p) = (integer(&peer_value), integer(&p));
    if peer <= BigInt::from(1u8) || peer >= &p - 1u8 {
        return Ok(refusal(
            "the peer's public value is outside 1 < value < p-1",
            chat_id,
        ));
    }
    let floor = BigInt::from(1u8) << (SECRET_BITS - 64);
    if peer < floor || peer > &p - &floor {
        return Ok(refusal(
            "the peer's public value is outside 2^1984 <= value <= p - 2^1984, the range small-subgroup and edge values fall into",
            chat_id,
        ));
    }
    let result = pow(peer, integer(&secret), p)?;
    let (_, bytes) = result.to_bytes_be();
    let mut key = [0; SECRET_BITS / 8];
    let start = key
        .len()
        .checked_sub(bytes.len())
        .ok_or("OverflowError: int too big to convert")?;
    key[start..].copy_from_slice(&bytes);
    Ok(json!({"key":{"$bytes":key.as_slice()}}))
}

/// Port of `verify_fingerprint` (`rust/adapters/handshake_adapter.py:35`).
pub fn verify_fingerprint(
    chat_id: i64,
    key: Vec<u8>,
    claimed: i64,
) -> Result<serde_json::Value, String> {
    if bool::from(Sha1::digest(&key)[12..20].ct_eq(&claimed.to_le_bytes())) {
        return Ok(json!({"ok":null}));
    }
    Ok(refusal(
        "the key fingerprint the peer published does not match the key this side derived: the exchange did not produce a shared key and the chat must be discarded",
        Some(chat_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_public_arithmetic_and_refuses_invalid_peer() -> Result<(), String> {
        assert_eq!(public_value(vec![2], vec![9], vec![23])?, "6");
        assert_eq!(public_value(vec![2], vec![255], vec![23])?, "12");
        assert_eq!(
            shared_key(vec![1], vec![2], vec![23], None)?["error_type"],
            "ParameterRejected"
        );
        assert_eq!(generate_secret()?["bits"], 2048);
        assert_eq!(
            verify_fingerprint(19, Vec::new(), 0)?["error_type"],
            "ParameterRejected"
        );
        Ok(())
    }
}
