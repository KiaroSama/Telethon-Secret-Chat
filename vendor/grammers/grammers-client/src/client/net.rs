// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![forbid(unsafe_code)]

use std::sync::Arc;

use grammers_crypto::aes::ige_encrypt;
use grammers_mtsender::{InvocationError, SenderPoolFatHandle};
use grammers_tl_types as tl;
use tokio::sync::Mutex;

use super::{Client, ClientConfiguration, ClientInner};

fn auth_key_tag(key: &[u8; 256]) -> [u8; 32] {
    let (chunks, _) = key.as_chunks::<32>();
    let (first, second) = (&chunks[0], &chunks[1]);
    let mut blocks = [0; 256];
    blocks[..16].copy_from_slice(b"grammers.dc.tag1");
    blocks[16..208].copy_from_slice(&key[64..]);
    ige_encrypt(&mut blocks, first, second);

    // Fold the final blocks under the swapped key and IV for this in-memory tag.
    let mut fold = [0; 48];
    fold[..16].copy_from_slice(b"grammers.dc.tag2");
    fold[16..].copy_from_slice(&blocks[224..]);
    ige_encrypt(&mut fold, second, first);
    let mut tag = [0; 32];
    tag.copy_from_slice(&fold[16..]);
    tag
}

#[cfg(test)]
mod tests {
    use super::auth_key_tag;

    #[test]
    fn dc_auth_cache_tag_matches_fixture_and_binds_every_key_bit() {
        let mut key: [u8; 256] = std::array::from_fn(|i| i as u8);
        let tag = auth_key_tag(&key);
        assert_eq!(
            tag,
            [
                0x75, 0x61, 0xe9, 0x28, 0x04, 0xb6, 0x93, 0x58, 0x48, 0xbd, 0x6b, 0x8b, 0xc3, 0x68,
                0x37, 0xa4, 0xd3, 0x0c, 0xe6, 0x31, 0x9f, 0x19, 0xb4, 0xc3, 0x21, 0x9c, 0x9c, 0x29,
                0xc6, 0x1d, 0x2b, 0x65,
            ]
        );
        assert_eq!(auth_key_tag(&key), tag);
        for part in tag.chunks_exact(16) {
            assert!(!key.windows(part.len()).any(|raw| raw == part));
        }
        for byte in 0..key.len() {
            for bit in 0..8 {
                key[byte] ^= 1 << bit;
                assert_ne!(
                    auth_key_tag(&key),
                    tag,
                    "unchanged at byte {byte} bit {bit}"
                );
                key[byte] ^= 1 << bit;
            }
        }
    }
}

/// Method implementations directly related with network connectivity.
impl Client {
    /// Creates and returns a new client instance upon successful connection to Telegram.
    ///
    /// If the session in the configuration did not have an authorization key, a new one
    /// will be created and the session will be saved with it.
    ///
    /// The connection will be initialized with the data from the input configuration.
    ///
    /// The [`grammers_mtsender::SenderPoolHandle`] does not keep a reference to the [`grammers_session::Session`]
    /// or `api_id`, but the [`grammers_mtsender::SenderPool`] itself does, so the latter is used as input to guarantee
    /// that the values are correctly shared between the pool and the client handles.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use grammers_client::Client;
    /// use grammers_session::storages::MemorySession; // avoid this storage outside tests!
    /// use grammers_mtsender::SenderPool;
    ///
    /// // Note: these are example values and are not actually valid.
    /// //       Obtain your own with the developer's phone at https://my.telegram.org.
    /// const API_ID: i32 = 932939;
    ///
    /// # async fn f() -> Result<(), Box<dyn std::error::Error>> {
    /// let session = Arc::new(MemorySession::default());
    /// let pool = SenderPool::new(Arc::clone(&session), API_ID);
    /// let client = Client::new(pool.handle);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(sender_pool: SenderPoolFatHandle) -> Self {
        Self::with_configuration(sender_pool, Default::default())
    }

    /// Like [`Self::new`] but with a custom [`ClientConfiguration`].
    pub fn with_configuration(
        sender_pool: SenderPoolFatHandle,
        configuration: ClientConfiguration,
    ) -> Self {
        // TODO Sender doesn't have a way to handle backpressure yet
        Self(Arc::new(ClientInner {
            session: sender_pool.session.clone(),
            api_id: sender_pool.api_id,
            handle: sender_pool.thin.clone(),
            auth_copied_to_dcs: Mutex::new(Vec::new()),
            configuration,
        }))
    }

    /// Invoke a raw API call. This directly sends the request to Telegram's servers.
    ///
    /// Using function definitions corresponding to a different layer is likely to cause the
    /// responses to the request to not be understood.
    ///
    /// <div class="warning">
    ///
    /// This method is **not** part of the stability guarantees of semantic
    /// versioning. It **may** break during *minor* version changes (but not on patch version
    /// changes). Use with care.
    ///
    /// </div>
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn f(client: grammers_client::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// use grammers_tl_types as tl;
    ///
    /// dbg!(client.invoke(&tl::functions::Ping { ping_id: 0 }).await?);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn invoke<R: tl::RemoteCall>(
        &self,
        request: &R,
    ) -> Result<R::Return, InvocationError> {
        let dc_id = self.0.session.home_dc_id()?;
        self.invoke_in_dc(dc_id, request).await
    }

    /// Like [`Self::invoke`], but in the specified DC.
    ///
    /// <div class="warning">
    ///
    /// This method is **not** part of the stability guarantees of semantic
    /// versioning. It **may** break during *minor* version changes (but not on patch version
    /// changes). Use with care.
    ///
    /// </div>
    pub async fn invoke_in_dc<R: tl::RemoteCall>(
        &self,
        dc_id: i32,
        request: &R,
    ) -> Result<R::Return, InvocationError> {
        self.0.handle.invoke_in_dc(dc_id, request).await
    }

    pub(crate) async fn copy_auth_to_dc(&self, target_dc_id: i32) -> Result<(), InvocationError> {
        let mut auth_copied_to_dcs = self.0.auth_copied_to_dcs.lock().await;
        let home_dc_id = self.0.session.home_dc_id()?;
        if target_dc_id == home_dc_id {
            return Ok(());
        }
        // Tie each cached import to its target key without retaining the key.
        let current_tag = || -> Result<Option<[u8; 32]>, InvocationError> {
            Ok(self
                .0
                .session
                .dc_option(target_dc_id)?
                .and_then(|option| option.auth_key.as_ref().map(auth_key_tag)))
        };
        for _ in 0..2 {
            let before = current_tag()?;
            if before.is_some_and(|tag| auth_copied_to_dcs.contains(&(target_dc_id, tag))) {
                return Ok(());
            }
            let tl::enums::auth::ExportedAuthorization::Authorization(exported_auth) = self
                .invoke(&tl::functions::auth::ExportAuthorization {
                    dc_id: target_dc_id,
                })
                .await?;
            self.invoke_in_dc(
                target_dc_id,
                &tl::functions::auth::ImportAuthorization {
                    id: exported_auth.id,
                    bytes: exported_auth.bytes,
                },
            )
            .await?;
            let after = current_tag()?;
            // Opening a fresh connection can create its key during import. Only
            // publish an import proven to span the same key on both sides.
            if let Some(tag) = after.filter(|_| before == after) {
                auth_copied_to_dcs.retain(|(dc, _)| *dc != target_dc_id);
                auth_copied_to_dcs.push((target_dc_id, tag));
                return Ok(());
            }
        }
        Err(InvocationError::Dropped(
            "target authorization changed during import",
        ))
    }
}
