//! Validated 2048-bit Diffie-Hellman exchange; parameters use grammers' safe-prime check.
use crate::{
    crypto::{SharedKey, random_bytes},
    error::{DhFailure, Error, Result},
};
use num_bigint::{BigInt, BigUint};
use num_traits::{One, ToPrimitive};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct DhConfig {
    g: u32,
    prime: BigUint,
}
type GoodParameters = std::sync::Mutex<Vec<(u32, Vec<u8>)>>;
impl DhConfig {
    pub fn new(g: u32, bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 256 {
            return Err(Error::DhFailure(DhFailure::Length));
        }
        Self::from_parameters(&BigInt::from(g), &value_from_bytes(bytes))
    }
    /// Validate externally supplied integer parameters before converting them to
    /// Telegram's narrow generator representation or doing primality work.
    pub fn from_parameters(generator: &BigInt, prime: &BigUint) -> Result<Self> {
        if prime.bits() != 2048 || *prime <= BigUint::one() << 2047 {
            return Err(Error::DhFailure(DhFailure::Length));
        }
        let g = generator
            .to_u32()
            .filter(|g| (2..=7).contains(g))
            .ok_or(Error::DhFailure(DhFailure::GeneratorRange))?;
        if !generator_is_quadratic_residue(g, prime) {
            return Err(Error::DhFailure(DhFailure::Residue));
        }
        static GOOD: std::sync::OnceLock<GoodParameters> = std::sync::OnceLock::new();
        let cache = GOOD.get_or_init(|| std::sync::Mutex::new(Vec::new()));
        let bytes = prime.to_bytes_be();
        if cache
            .lock()
            .map_err(|_| Error::DhParameters)?
            .iter()
            .any(|(known_g, known)| *known_g == g && *known == bytes)
        {
            return Ok(Self {
                g,
                prime: prime.clone(),
            });
        }
        if !grammers_crypto::check_dh_prime(prime, g as i32).map_err(|_| Error::Randomness)? {
            return Err(Error::DhFailure(DhFailure::UnsafePrime));
        }
        let mut good = cache.lock().map_err(|_| Error::DhParameters)?;
        if good.len() >= 8 {
            good.remove(0);
        }
        good.push((g, bytes));
        Ok(Self {
            g,
            prime: prime.clone(),
        })
    }
    pub fn prime_bytes(&self) -> Vec<u8> {
        self.prime.to_bytes_be()
    }
    pub fn generator(&self) -> u32 {
        self.g
    }
    pub fn check_peer(&self, bytes: &[u8]) -> Result<BigUint> {
        if bytes.len() > 256 {
            return Err(Error::PeerValue);
        }
        self.check_peer_integer(&BigInt::from_bytes_be(num_bigint::Sign::Plus, bytes))
    }
    /// Accept signed integers from application decoders, rejecting negative values.
    pub fn check_peer_integer(&self, value: &BigInt) -> Result<BigUint> {
        let value = value.to_biguint().ok_or(Error::PeerValue)?;
        let floor = BigUint::one() << 1984;
        if value < floor || value > &self.prime - &floor {
            return Err(Error::PeerValue);
        }
        Ok(value)
    }
    pub fn public_value(&self, secret: &SecretExponent) -> Vec<u8> {
        BigUint::from(self.g)
            .modpow(&BigUint::from_bytes_be(&secret.0[..]), &self.prime)
            .to_bytes_be()
    }
    pub fn shared_key(&self, peer: &[u8], secret: &SecretExponent) -> Result<SharedKey> {
        let peer = self.check_peer(peer)?;
        let value = peer.modpow(&BigUint::from_bytes_be(&secret.0[..]), &self.prime);
        let raw = Zeroizing::new(value.to_bytes_be());
        let mut padded = Zeroizing::new([0u8; 256]);
        padded[256 - raw.len()..].copy_from_slice(&raw);
        SharedKey::from_slice(&padded[..])
    }
}

/// BigUint's exponentiation is not constant-time. The private exponent remains
/// explicitly zeroized at rest; applications requiring side-channel isolation
/// must isolate the process (see the protocol documentation).
pub struct SecretExponent(Zeroizing<[u8; 256]>);
impl SecretExponent {
    pub fn generate() -> Result<Self> {
        let mut bytes = random_bytes::<256>()?;
        bytes[0] |= 0x80;
        Ok(Self(Zeroizing::new(bytes)))
    }
    pub fn from_bytes(bytes: [u8; 256]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    pub fn expose(&self) -> &[u8; 256] {
        &self.0
    }
}
pub fn verify_fingerprint(key: &SharedKey, claimed: i64) -> Result<()> {
    if key.fingerprint() == claimed {
        Ok(())
    } else {
        Err(Error::Fingerprint)
    }
}
impl Clone for SecretExponent {
    fn clone(&self) -> Self {
        Self(Zeroizing::new(*self.0))
    }
}
impl serde::Serialize for SecretExponent {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let value = Zeroizing::new(hex::encode(*self.0));
        serializer.serialize_str(&value)
    }
}
impl<'de> serde::Deserialize<'de> for SecretExponent {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Zeroizing::new(<String as serde::Deserialize>::deserialize(deserializer)?);
        let mut bytes = Zeroizing::new([0u8; 256]);
        hex::decode_to_slice(&*value, &mut bytes[..])
            .map_err(|_| serde::de::Error::custom("Invalid private exponent"))?;
        Ok(Self(bytes))
    }
}

/// Decode Telegram's unsigned big-endian public integer encoding.
pub fn value_from_bytes(bytes: &[u8]) -> BigUint {
    BigUint::from_bytes_be(bytes)
}
pub fn is_probable_prime(value: &BigUint) -> Result<bool> {
    glass_pumpkin::prime::check(value).map_err(|_| Error::Randomness)
}
pub fn is_safe_prime(value: &BigUint) -> Result<bool> {
    glass_pumpkin::safe_prime::check(value).map_err(|_| Error::Randomness)
}
pub fn generator_is_quadratic_residue(g: u32, prime: &BigUint) -> bool {
    let residue = |modulus: u32| (prime % modulus).to_u32().unwrap_or(u32::MAX);
    match g {
        2 => residue(8) == 7,
        3 => residue(3) == 2,
        4 => true,
        5 => matches!(residue(5), 1 | 4),
        6 => matches!(residue(24), 19 | 23),
        7 => matches!(residue(7), 3 | 5 | 6),
        _ => false,
    }
}
