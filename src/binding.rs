//! A stable server-confirmed account/device/DC binding. It deliberately has no
//! Debug implementation and never serializes the underlying MTProto auth key.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationBinding {
    user_id: i64,
    authorization_hash: i64,
    dc_id: i32,
    generation: String,
    #[serde(default)]
    session_key: bool,
}
impl AuthorizationBinding {
    /// Validate parts obtained by an explicitly trusted transport/session owner.
    /// This constructor alone does not query or authenticate a Telegram client.
    pub fn from_parts(
        user_id: i64,
        authorization_hash: i64,
        dc_id: i32,
        generation: String,
    ) -> Result<Self> {
        let value = Self {
            user_id,
            authorization_hash,
            dc_id,
            generation,
            session_key: false,
        };
        value.validate()?;
        Ok(value)
    }
    /// Bind ingress to SHA-1(auth_key)[12..20] of the original session.
    /// This constructor does not authenticate the supplied values.
    pub fn from_session_parts(
        user_id: i64,
        auth_key_id: i64,
        dc_id: i32,
        generation: String,
    ) -> Result<Self> {
        let mut value = Self::from_parts(user_id, auth_key_id, dc_id, generation)?;
        value.session_key = true;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if self.user_id <= 0
            || self.authorization_hash == 0
            || !(1..=1000).contains(&self.dc_id)
            || self.generation.is_empty()
            || self.generation.len() > 128
        {
            return Err(Error::Authorization);
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"kiaro-grammers-binding-v1\0");
        if self.session_key {
            hash.update(b"session-key\0");
        }
        hash.update(self.user_id.to_le_bytes());
        hash.update(self.authorization_hash.to_le_bytes());
        hash.update(self.dc_id.to_le_bytes());
        hash.update(self.generation.as_bytes());
        hex::encode(hash.finalize())
    }
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn dc_id(&self) -> i32 {
        self.dc_id
    }
    pub fn auth_key_id(&self) -> Option<i64> {
        self.session_key.then_some(self.authorization_hash)
    }
    pub fn generation(&self) -> &str {
        &self.generation
    }
}
