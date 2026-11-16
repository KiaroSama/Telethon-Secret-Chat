//! A non-Debug, zeroizing encoded protected record for private manager state.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretRecord(Vec<u8>);
impl SecretRecord {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl Serialize for SecretRecord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let hex = Zeroizing::new(hex::encode(&self.0));
        serializer.serialize_str(&hex)
    }
}
impl<'de> Deserialize<'de> for SecretRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let hex = Zeroizing::new(String::deserialize(deserializer)?);
        if hex.len() > 8 * 1024 * 1024 {
            return Err(serde::de::Error::custom(
                "Protected record exceeds its size limit",
            ));
        }
        let bytes =
            hex::decode(&*hex).map_err(|_| serde::de::Error::custom("Invalid protected record"))?;
        Ok(Self(bytes))
    }
}
