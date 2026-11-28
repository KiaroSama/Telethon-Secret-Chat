//! Stable binding to a caller's verified Telegram user and local authorization key.
use crate::error::{Error, Result};
use crate::transport::compute_v2_authorization_identity;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationBinding {
    user_id: i64,
    dc_id: i32,
    auth_key_tag: [u8; 32],
    generation: String,
}

impl AuthorizationBinding {
    /// Construct a binding from parts verified by the caller. Obtain the DC and
    /// tag from `Client::authorization_key_tag()` around authorization verification.
    pub fn v2(
        user_id: i64,
        dc_id: i32,
        auth_key_tag: [u8; 32],
        generation: impl Into<String>,
    ) -> Result<Self> {
        let binding = Self {
            user_id,
            dc_id,
            auth_key_tag,
            generation: generation.into(),
        };
        binding.validate()?;
        Ok(binding)
    }

    /// The second argument is the full local key tag, never a management hash.
    pub fn from_parts(
        user_id: i64,
        auth_key_tag: [u8; 32],
        dc_id: i32,
        generation: String,
    ) -> Result<Self> {
        Self::v2(user_id, dc_id, auth_key_tag, generation)
    }

    /// A v1 key ID is insufficient to construct a v2 binding. Use [`Self::v2`]
    /// with the original client's tag, or `binding_migration` for an offline store.
    pub fn from_session_parts(
        _user_id: i64,
        _auth_key_id: i64,
        _dc_id: i32,
        _generation: String,
    ) -> Result<Self> {
        Err(Error::Authorization)
    }

    pub fn validate(&self) -> Result<()> {
        if self.user_id <= 0
            || !(1..=1000).contains(&self.dc_id)
            || self.generation.trim().is_empty()
            || self.generation.len() > 128
        {
            return Err(Error::Authorization);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> String {
        compute_v2_authorization_identity(
            self.user_id,
            self.auth_key_tag,
            self.dc_id,
            &self.generation,
        )
    }

    pub fn digest(&self) -> String {
        self.fingerprint()
    }

    pub fn user_id(&self) -> i64 {
        self.user_id
    }

    pub fn dc_id(&self) -> i32 {
        self.dc_id
    }

    /// A v2 binding does not carry the old truncated SHA-1 key ID.
    pub fn auth_key_id(&self) -> Option<i64> {
        None
    }

    pub fn auth_key_tag(&self) -> Option<[u8; 32]> {
        Some(self.auth_key_tag)
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub(crate) fn check_client_key(&self, client: &grammers_client::Client) -> Result<()> {
        if client
            .authorization_key_tag()
            .map_err(|_| Error::Authorization)?
            != Some((self.dc_id, self.auth_key_tag))
        {
            return Err(Error::Authorization);
        }
        Ok(())
    }
}
