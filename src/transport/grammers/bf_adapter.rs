use super::{Client, GrammersTransport, peer, tl};
use crate::{
    binding::AuthorizationBinding,
    dh::DhConfig,
    error::{Error, Result},
    files::FileKey,
    ingress::DurableIngress,
    manager::transport::{BoxFuture, RpcFailure, Sent, Transport, classify_invocation_error},
    media::{MediaReference, SourceReader},
    protocol::SendEffect,
};
use grammers_tl_types::Deserializable;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

impl GrammersTransport {
    /// Wrap the caller's client for the managed API. The existing core API uses `connect`.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            core_binding: None,
            managed_authorization: Arc::default(),
            io: Arc::default(),
            ingress: None,
            chats: Arc::default(),
            dh: Arc::default(),
        }
    }

    /// Associate an ingress journal without granting proof of the original sender pool.
    /// Use `DurableIngress::transport` to obtain a transport qualified for strong custody.
    pub fn with_durable_ingress(client: Client, ingress: Arc<DurableIngress>) -> Self {
        let mut transport = Self::new(client);
        transport.ingress = Some((ingress, false));
        transport
    }

    pub(crate) fn from_verified_ingress(client: Client, ingress: Arc<DurableIngress>) -> Self {
        let mut transport = Self::new(client);
        transport.ingress = Some((ingress, true));
        transport
    }

    /// Verify the existing account, home data center, and MTProto key without reading the key.
    pub async fn verified_binding(&self, generation: &str) -> Result<AuthorizationBinding> {
        if let Some((ingress, true)) = &self.ingress {
            let binding = self.checked_ingress(ingress)?;
            if generation != binding.generation() {
                return Err(Error::Authorization);
            }
            return Ok(binding);
        }
        self.check_bound_client()?;
        let authorization = super::query_authorization(&self.client)
            .await
            .map_err(|_| Error::Authorization)?;
        if let Some(core) = &self.core_binding
            && authorization != core.authorization
        {
            return Err(Error::Authorization);
        }
        let binding = AuthorizationBinding::v2(
            authorization.user_id,
            authorization.dc_id,
            authorization.key_tag,
            generation,
        )?;
        binding.check_client_key(&self.client)?;
        if self
            .managed_authorization
            .get_or_init(|| authorization.clone())
            != &authorization
        {
            return Err(Error::Authorization);
        }
        Ok(binding)
    }

    /// Require proof that this exact client shares the journal's original ACK boundary.
    pub fn require_strong_ingress(&self) -> Result<()> {
        let (ingress, _) = self.ingress.as_ref().ok_or(Error::IngressUnavailable)?;
        self.checked_ingress(ingress).map(|_| ())
    }

    pub(crate) fn checked_ingress(
        &self,
        expected: &Arc<DurableIngress>,
    ) -> Result<AuthorizationBinding> {
        let (ingress, true) = self.ingress.as_ref().ok_or(Error::IngressUnavailable)? else {
            return Err(Error::IngressUnavailable);
        };
        if !Arc::ptr_eq(ingress, expected) {
            return Err(Error::Authorization);
        }
        let binding = ingress.verified_binding()?;
        binding.check_client_key(&self.client)?;
        Ok(binding)
    }

    async fn checked_rpc<R: tl::RemoteCall>(
        &self,
        request: &R,
    ) -> std::result::Result<R::Return, RpcFailure> {
        self.check_bound_client()
            .map_err(|_| RpcFailure::Cancelled)?;
        let response = self.client.invoke(request).await;
        self.check_bound_client()
            .map_err(|_| RpcFailure::Cancelled)?;
        response.map_err(classify_invocation_error)
    }

    fn check_bound_client(&self) -> Result<()> {
        if self.ingress.is_some() {
            self.require_strong_ingress()?;
        }
        if self.core_binding.is_some() {
            self.require_current_key()
                .map_err(|_| Error::Authorization)?;
        } else if let Some(pinned) = self.managed_authorization.get() {
            let key = super::authorization_key_snapshot(&self.client)
                .map_err(|_| Error::Authorization)?;
            if key != (pinned.dc_id, pinned.key_tag) {
                return Err(Error::Authorization);
            }
        }
        Ok(())
    }

    fn transfer_guard(&self) -> crate::media::TransferGuard {
        let transport = self.clone();
        Arc::new(move || transport.check_bound_client())
    }

    pub async fn dh_config(&self) -> Result<DhConfig> {
        Transport::dh_config(self).await
    }

    pub async fn request_encryption(
        &self,
        user: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> Result<tl::enums::EncryptedChat> {
        Transport::request(self, user, random_id, g_a)
            .await
            .map_err(|_| Error::Transport)
    }

    pub async fn accept_encryption(
        &self,
        id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        fingerprint: i64,
    ) -> Result<tl::enums::EncryptedChat> {
        Transport::accept(self, id, access_hash, g_b, fingerprint)
            .await
            .map_err(|_| Error::Transport)
    }

    pub async fn discard(&self, id: i32, delete_history: bool) -> Result<()> {
        Transport::discard(self, id, delete_history).await
    }

    pub async fn send(&self, effect: &SendEffect) -> Result<()> {
        Transport::send(self, effect)
            .await
            .map(|_| ())
            .map_err(|_| Error::Transport)
    }
}

impl Transport for GrammersTransport {
    fn drain_io(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.io.drain())
    }
    fn resume_io(&self) -> Result<()> {
        self.io.resume()
    }

    fn requires_verified_binding(&self) -> bool {
        true
    }

    fn authorization_digest(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move {
            let binding = if let Some((ingress, _)) = &self.ingress {
                self.checked_ingress(ingress)?
            } else {
                let generation = self
                    .core_binding
                    .as_ref()
                    .map_or("session", |core| core.generation.as_str());
                self.verified_binding(generation).await?
            };
            Ok(Some(binding.digest()))
        })
    }

    fn dh_config(&self) -> BoxFuture<'_, Result<DhConfig>> {
        Box::pin(async move {
            match self
                .checked_rpc(&tl::functions::messages::GetDhConfig {
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
        })
    }
    fn request(
        &self,
        user_id: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>> {
        Box::pin(async move {
            self.checked_rpc(&tl::functions::messages::RequestEncryption {
                user_id,
                random_id,
                g_a,
            })
            .await
        })
    }
    fn accept(
        &self,
        id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        key_fingerprint: i64,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>> {
        Box::pin(async move {
            self.checked_rpc(&tl::functions::messages::AcceptEncryption {
                peer: peer(id, access_hash),
                g_b,
                key_fingerprint,
            })
            .await
        })
    }
    fn discard(&self, id: i32, delete_history: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if self
                .checked_rpc(&tl::functions::messages::DiscardEncryption {
                    delete_history,
                    chat_id: id,
                })
                .await
                .map_err(|_| Error::Transport)?
            {
                Ok(())
            } else {
                Err(Error::Transport)
            }
        })
    }
    fn send<'a>(
        &'a self,
        effect: &'a SendEffect,
    ) -> BoxFuture<'a, std::result::Result<Sent, RpcFailure>> {
        Box::pin(async move {
            let row = &effect.outgoing;
            let peer = peer(effect.chat_id, effect.access_hash);
            let silent = if row.service || row.body.is_empty() {
                false
            } else {
                matches!(
                    crate::framing::unwrap(&row.body).map_err(|_| RpcFailure::Permanent)?.message,
                    crate::enums::DecryptedMessage::Message(message) if message.silent
                )
            };
            let response = if let Some(file) = &row.file {
                let file = tl::enums::InputEncryptedFile::from_bytes(file)
                    .map_err(|_| RpcFailure::Permanent)?;
                self.checked_rpc(&tl::functions::messages::SendEncryptedFile {
                    silent,
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                    file,
                })
                .await
            } else if row.service {
                self.checked_rpc(&tl::functions::messages::SendEncryptedService {
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                })
                .await
            } else {
                self.checked_rpc(&tl::functions::messages::SendEncrypted {
                    silent,
                    peer,
                    random_id: row.random_id,
                    data: row.frame.clone(),
                })
                .await
            };
            Ok(Sent {
                file: match response? {
                    tl::enums::messages::SentEncryptedMessage::SentEncryptedFile(sent) => {
                        match sent.file {
                            tl::enums::EncryptedFile::File(file) => Some(file),
                            _ => None,
                        }
                    }
                    _ => None,
                },
            })
        })
    }
    fn upload<'a>(
        &'a self,
        source: &'a mut SourceReader,
        key: &'a FileKey,
    ) -> BoxFuture<'a, Result<tl::enums::InputEncryptedFile>> {
        Box::pin(async move {
            self.check_bound_client()?;
            let file = self
                .io
                .scope(crate::media::upload_file_guarded(
                    self.client(),
                    source,
                    key,
                    self.transfer_guard(),
                ))
                .await?;
            self.check_bound_client()?;
            Ok(file)
        })
    }
    fn download_bytes<'a>(
        &'a self,
        reference: &'a MediaReference,
        maximum: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            self.check_bound_client()?;
            let mut bytes = self
                .io
                .scope(crate::media::download_bytes_guarded(
                    self.client(),
                    reference,
                    maximum,
                    self.transfer_guard(),
                ))
                .await?;
            self.check_bound_client()?;
            Ok(std::mem::take(&mut *bytes))
        })
    }
    fn download<'a>(
        &'a self,
        reference: &'a MediaReference,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<PathBuf>> {
        Box::pin(async move {
            self.check_bound_client()?;
            let path = self
                .io
                .scope(crate::media::download_file_guarded(
                    self.client(),
                    reference,
                    path,
                    self.transfer_guard(),
                ))
                .await?;
            self.check_bound_client()?;
            Ok(path)
        })
    }
}
