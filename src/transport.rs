//! Same-client grammers adapter. It never creates a client, signs in, or opens a
//! second authorization. The application continues to own its update stream.
use crate::{
    dh::DhConfig,
    error::{Error, Result},
    protocol::SendEffect,
};
use grammers_client::Client;
use grammers_tl_types::{self as tl, Deserializable};
#[derive(Clone)]
pub struct GrammersTransport {
    client: Client,
}
impl GrammersTransport {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
    pub fn client(&self) -> &Client {
        &self.client
    }
    pub async fn dh_config(&self) -> Result<DhConfig> {
        match self
            .client
            .invoke(&tl::functions::messages::GetDhConfig {
                version: 0,
                random_length: 0,
            })
            .await
            .map_err(|_| Error::Transport)?
        {
            tl::enums::messages::DhConfig::Config(config) => {
                DhConfig::new(config.g as u32, &config.p)
            }
            _ => Err(Error::DhParameters),
        }
    }
    pub async fn request_encryption(
        &self,
        user: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> Result<tl::enums::EncryptedChat> {
        self.client
            .invoke(&tl::functions::messages::RequestEncryption {
                user_id: user,
                random_id,
                g_a,
            })
            .await
            .map_err(|_| Error::Transport)
    }
    pub async fn accept_encryption(
        &self,
        id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        key_fingerprint: i64,
    ) -> Result<tl::enums::EncryptedChat> {
        self.client
            .invoke(&tl::functions::messages::AcceptEncryption {
                peer: peer(id, access_hash),
                g_b,
                key_fingerprint,
            })
            .await
            .map_err(|_| Error::Transport)
    }
    pub async fn discard(&self, id: i32, delete_history: bool) -> Result<()> {
        if !self
            .client
            .invoke(&tl::functions::messages::DiscardEncryption {
                delete_history,
                chat_id: id,
            })
            .await
            .map_err(|_| Error::Transport)?
        {
            return Err(Error::Transport);
        }
        Ok(())
    }
    pub async fn send(&self, effect: &SendEffect) -> Result<()> {
        let row = &effect.outgoing;
        let peer = peer(effect.chat_id, effect.access_hash);
        if let Some(file) = &row.file {
            let file = tl::enums::InputEncryptedFile::from_bytes(file).map_err(|_| Error::Media)?;
            self.client
                .invoke(&tl::functions::messages::SendEncryptedFile {
                    silent: false,
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                    file,
                })
                .await
                .map_err(|_| Error::Transport)?;
        } else if row.service {
            self.client
                .invoke(&tl::functions::messages::SendEncryptedService {
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                })
                .await
                .map_err(|_| Error::Transport)?;
        } else {
            self.client
                .invoke(&tl::functions::messages::SendEncrypted {
                    silent: false,
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                })
                .await
                .map_err(|_| Error::Transport)?;
        }
        Ok(())
    }
    /// Receiving an ordinary Raw update does not establish durable pre-ack custody.
    pub fn require_strong_ingress(&self) -> Result<()> {
        Err(Error::IngressUnavailable)
    }
}
fn peer(id: i32, access_hash: i64) -> tl::enums::InputEncryptedChat {
    tl::types::InputEncryptedChat {
        chat_id: id,
        access_hash,
    }
    .into()
}
