//! Diffie–Hellman parameter validation and the lossless migration boundary.

use num_bigint::{BigInt, BigUint};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex};

pub const DH_BITS: usize = 2048;
const ROUNDS: i64 = 15;
const SMALL_PRIMES: [u8; 15] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47];
static SAFE_PRIMES: Mutex<VecDeque<(BigInt, bool)>> = Mutex::new(VecDeque::new());

fn integer(raw: &[u8]) -> BigInt {
    BigInt::from_signed_bytes_be(raw)
}

fn random_below(bound: &BigUint) -> Result<BigUint, String> {
    let mut raw = bound.to_bytes_be();
    let first = raw.first().ok_or("RuntimeError: empty random range")?;
    let mask = u8::MAX >> first.leading_zeros();
    loop {
        getrandom::fill(&mut raw).map_err(|_| "RuntimeError: OS randomness unavailable")?;
        raw[0] &= mask;
        let value = BigUint::from_bytes_be(&raw);
        if &value < bound {
            return Ok(value);
        }
    }
}

fn probable(n: &BigInt, rounds: i64) -> Result<bool, String> {
    if n < &BigInt::from(2u8) {
        return Ok(false);
    }
    // This value is positive after the range check, and is public protocol data.
    let n = n.magnitude();
    for small in SMALL_PRIMES {
        if n % small == BigUint::default() {
            return Ok(*n == BigUint::from(small));
        }
    }
    let one = BigUint::from(1u8);
    let last = n - 1u8;
    let count = last.trailing_zeros().unwrap_or_default();
    let exponent = &last >> count;
    let bound = n - 3u8;
    'witness: for _ in 0..rounds {
        let a = random_below(&bound)? + 2u8;
        let mut value = a.modpow(&exponent, n);
        if value == one || value == last {
            continue;
        }
        for _ in 1..count {
            value = (&value * &value) % n;
            if value == last {
                continue 'witness;
            }
        }
        return Ok(false);
    }
    Ok(true)
}

fn safe_prime(p: &BigInt) -> Result<bool, String> {
    let mut cache = SAFE_PRIMES
        .lock()
        .map_err(|_| "RuntimeError: prime cache unavailable")?;
    if let Some(i) = cache.iter().position(|(n, _)| n == p)
        && let Some(entry) = cache.remove(i)
    {
        let result = entry.1;
        cache.push_back(entry);
        return Ok(result);
    }
    let result = probable(p, ROUNDS)? && probable(&((p - 1u8) >> 1u8), ROUNDS)?;
    if cache.len() == 8 {
        cache.pop_front();
    }
    cache.push_back((p.clone(), result));
    Ok(result)
}

fn residue(g: &BigInt, p: &BigInt) -> bool {
    let (modulus, allowed): (u8, &[u8]) = match u8::try_from(g) {
        Ok(2) => (8, &[7]),
        Ok(3) => (3, &[2]),
        Ok(4) => return true,
        Ok(5) => (5, &[1, 4]),
        Ok(6) => (24, &[19, 23]),
        Ok(7) => (7, &[3, 5, 6]),
        _ => return false,
    };
    let rem = ((p % modulus) + modulus) % modulus;
    u8::try_from(rem).is_ok_and(|v| allowed.contains(&v))
}

fn validation(reason: Option<&str>, chat: Option<Vec<u8>>) -> Value {
    let Some(reason) = reason else {
        return json!({"ok": true, "value": null});
    };
    let message = format!("refused the key exchange: {reason}");
    let message = chat.map_or_else(
        || message.clone(),
        |id| format!("[chat {}] {message}", integer(&id)),
    );
    json!({"ok": false, "error_type": "grammers_secret_chat.errors.ParameterRejected", "reason": reason, "message": message})
}

pub fn is_probable_prime(n: Vec<u8>, rounds: Option<i64>) -> Result<bool, String> {
    probable(&integer(&n), rounds.unwrap_or(ROUNDS))
}

pub fn is_safe_prime(p: Vec<u8>) -> Result<bool, String> {
    safe_prime(&integer(&p))
}

pub fn generator_is_quadratic_residue(g: Vec<u8>, p: Vec<u8>) -> Result<bool, String> {
    Ok(residue(&integer(&g), &integer(&p)))
}

pub fn check_config(g: Vec<u8>, p: Vec<u8>, chat_id: Option<Vec<u8>>) -> Result<Value, String> {
    let (g, p) = (integer(&g), integer(&p));
    let reason = if p.bits() != DH_BITS as u64 || p <= (BigInt::from(1u8) << (DH_BITS - 1)) {
        Some("the prime is not 2048 bits: the protocol requires 2^2047 < p < 2^2048")
    } else if g <= BigInt::from(1u8) || g >= &p - 1u8 {
        Some("the generator is outside 1 < g < p-1")
    } else if !residue(&g, &p) {
        Some(
            "the generator does not generate the subgroup of order (p-1)/2 for this prime, or is not one of the six the protocol defines",
        )
    } else if !safe_prime(&p)? {
        Some("the prime is not a safe prime: p and (p-1)/2 must both be prime")
    } else {
        None
    };
    Ok(validation(reason, chat_id))
}

pub fn check_peer_value(
    value: Vec<u8>,
    p: Vec<u8>,
    chat_id: Option<Vec<u8>>,
) -> Result<Value, String> {
    let (value, p) = (integer(&value), integer(&p));
    let floor = BigInt::from(1u8) << (DH_BITS - 64);
    let reason = if value <= BigInt::from(1u8) || value >= &p - 1u8 {
        Some("the peer's public value is outside 1 < value < p-1")
    } else if value < floor || value > &p - &floor {
        Some(
            "the peer's public value is outside 2^1984 <= value <= p - 2^1984, the range small-subgroup and edge values fall into",
        )
    } else {
        None
    };
    Ok(validation(reason, chat_id))
}

pub fn value_from_bytes(raw: Vec<u8>) -> Result<String, String> {
    Ok(BigUint::from_bytes_be(&raw).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_prime_from_composite_and_checks_both_safe_prime_factors() {
        assert_eq!(is_probable_prime(vec![97], None), Ok(true));
        assert_eq!(is_probable_prime(vec![91], None), Ok(false));
        assert_eq!(is_safe_prime(vec![11]), Ok(true));
        assert_eq!(is_safe_prime(vec![13]), Ok(false));
    }

    #[test]
    fn rejects_parameters_without_echoing_them() {
        assert_eq!(generator_is_quadratic_residue(vec![2], vec![15]), Ok(true));
        let response = check_config(vec![2], vec![23], Some(vec![7]));
        assert!(
            matches!(&response, Ok(v) if v["error_type"] == "grammers_secret_chat.errors.ParameterRejected" && v["message"].as_str().is_some_and(|s| s.starts_with("[chat 7]") && !s.contains("23")))
        );
        assert!(matches!(check_peer_value(vec![1], vec![23], None), Ok(v) if v["ok"] == false));
        assert_eq!(value_from_bytes(vec![0, 1, 0]), Ok(String::from("256")));
    }

    #[test]
    fn accepts_rfc_3526_group_14() -> Result<(), String> {
        let digits = concat!(
            "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
            "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
            "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
            "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
            "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
            "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
            "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
            "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
        );
        let p = num_bigint::BigInt::parse_bytes(digits.as_bytes(), 16)
            .ok_or("Invalid RFC 3526 group 14 test fixture")?;
        assert_eq!(
            check_config(vec![2], p.to_signed_bytes_be(), None)?["ok"],
            true
        );
        let floor = num_bigint::BigInt::from(1u8) << 1984usize;
        let ceiling = &p - &floor;
        for value in [&floor, &ceiling] {
            assert_eq!(
                check_peer_value(value.to_signed_bytes_be(), p.to_signed_bytes_be(), None)?["ok"],
                true
            );
        }
        Ok(())
    }
}
