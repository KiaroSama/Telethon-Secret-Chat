//! All transport failures are classified before entering manager logs/events.
use crate::{
    dh::DhConfig,
    error::Result,
    files::FileKey,
    media::{MediaReference, SourceReader},
    protocol::SendEffect,
};
use grammers_tl_types::{self as tl, Deserializable, Serializable};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermanentError {
    DataInvalid,
    DataTooLong,
    FileEmpty,
    Md5ChecksumInvalid,
}
impl PermanentError {
    /// Static Telegram rejection class name, without the RPC's error payload.
    pub fn name(self) -> &'static str {
        match self {
            Self::DataInvalid => "DataInvalidError",
            Self::DataTooLong => "DataTooLongError",
            Self::FileEmpty => "FileEmptyError",
            Self::Md5ChecksumInvalid => "Md5ChecksumInvalidError",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcFailure {
    Transient,
    Cancelled,
    ChatEnded,
    Permanent,
    Rejected(PermanentError),
    AlreadyAccepted,
    Declined,
}
#[derive(Default)]
pub struct Sent {
    pub file: Option<tl::types::EncryptedFile>,
}
pub trait Transport: Clone + Send + Sync + 'static {
    fn requires_verified_binding(&self) -> bool {
        false
    }

    /// Custom transports may bind independently; native grammers verifies its live authorization.
    fn authorization_digest(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }

    fn dh_config(&self) -> BoxFuture<'_, Result<DhConfig>>;
    fn request(
        &self,
        peer: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>>;
    fn accept(
        &self,
        id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        fingerprint: i64,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>>;
    fn discard(&self, id: i32, delete_history: bool) -> BoxFuture<'_, Result<()>>;
    fn send<'a>(
        &'a self,
        effect: &'a SendEffect,
    ) -> BoxFuture<'a, std::result::Result<Sent, RpcFailure>>;
    fn upload<'a>(
        &'a self,
        source: &'a mut SourceReader,
        key: &'a FileKey,
    ) -> BoxFuture<'a, Result<tl::enums::InputEncryptedFile>>;
    fn download_bytes<'a>(
        &'a self,
        reference: &'a MediaReference,
        maximum: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>>>;
    fn download<'a>(
        &'a self,
        reference: &'a MediaReference,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<PathBuf>>;
}
/// Classify a native SDK invocation failure without retaining its RPC payload.
pub fn classify_invocation_error(error: grammers_client::InvocationError) -> RpcFailure {
    if let grammers_client::InvocationError::Rpc(rpc) = error {
        match rpc.name.as_str() {
            "CHAT_ID_INVALID"
            | "ENCRYPTION_DECLINED"
            | "ENCRYPTION_ID_INVALID"
            | "USER_DELETED" => RpcFailure::ChatEnded,
            "DATA_INVALID" => RpcFailure::Rejected(PermanentError::DataInvalid),
            "DATA_TOO_LONG" => RpcFailure::Rejected(PermanentError::DataTooLong),
            "FILE_EMPTY" | "FILE_EMTPY" => RpcFailure::Rejected(PermanentError::FileEmpty),
            "MD5_CHECKSUM_INVALID" => RpcFailure::Rejected(PermanentError::Md5ChecksumInvalid),
            "ENCRYPTION_ALREADY_ACCEPTED" => RpcFailure::AlreadyAccepted,
            "ENCRYPTION_ALREADY_DECLINED" => RpcFailure::Declined,
            _ => RpcFailure::Transient,
        }
    } else {
        RpcFailure::Transient
    }
}
fn peer(id: i32, access_hash: i64) -> tl::enums::InputEncryptedChat {
    tl::types::InputEncryptedChat {
        chat_id: id,
        access_hash,
    }
    .into()
}
impl Transport for crate::transport::GrammersTransport {
    fn requires_verified_binding(&self) -> bool {
        true
    }

    fn authorization_digest(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move { Ok(Some(self.verified_binding("session").await?.digest())) })
    }

    fn dh_config(&self) -> BoxFuture<'_, Result<DhConfig>> {
        Box::pin(crate::transport::GrammersTransport::dh_config(self))
    }
    fn request(
        &self,
        user_id: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>> {
        Box::pin(async move {
            self.client()
                .invoke(&tl::functions::messages::RequestEncryption {
                    user_id,
                    random_id,
                    g_a,
                })
                .await
                .map_err(classify_invocation_error)
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
            self.client()
                .invoke(&tl::functions::messages::AcceptEncryption {
                    peer: peer(id, access_hash),
                    g_b,
                    key_fingerprint,
                })
                .await
                .map_err(classify_invocation_error)
        })
    }
    fn discard(&self, id: i32, delete_history: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(crate::transport::GrammersTransport::discard(
            self,
            id,
            delete_history,
        ))
    }
    fn send<'a>(
        &'a self,
        effect: &'a SendEffect,
    ) -> BoxFuture<'a, std::result::Result<Sent, RpcFailure>> {
        Box::pin(async move {
            let row = &effect.outgoing;
            let peer = peer(effect.chat_id, effect.access_hash);
            let response = if let Some(file) = &row.file {
                let file = tl::enums::InputEncryptedFile::from_bytes(file)
                    .map_err(|_| RpcFailure::Permanent)?;
                self.client()
                    .invoke(&tl::functions::messages::SendEncryptedFile {
                        silent: false,
                        peer,
                        random_id: row.random_id,
                        data: row.frame.clone(),
                        file,
                    })
                    .await
            } else if row.service {
                self.client()
                    .invoke(&tl::functions::messages::SendEncryptedService {
                        peer,
                        random_id: row.random_id,
                        data: row.frame.clone(),
                    })
                    .await
            } else {
                self.client()
                    .invoke(&tl::functions::messages::SendEncrypted {
                        silent: false,
                        peer,
                        random_id: row.random_id,
                        data: row.frame.clone(),
                    })
                    .await
            };
            Ok(Sent {
                file: match response.map_err(classify_invocation_error)? {
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
        Box::pin(crate::media::upload_file(self.client(), source, key))
    }
    fn download_bytes<'a>(
        &'a self,
        reference: &'a MediaReference,
        maximum: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(crate::media::download_bytes(
            self.client(),
            reference,
            maximum,
        ))
    }
    fn download<'a>(
        &'a self,
        reference: &'a MediaReference,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<PathBuf>> {
        Box::pin(crate::media::download_file(self.client(), reference, path))
    }
}
pub(crate) fn file_handle(file: tl::types::EncryptedFile) -> Vec<u8> {
    tl::enums::InputEncryptedFile::from(tl::types::InputEncryptedFile {
        id: file.id,
        access_hash: file.access_hash,
    })
    .to_bytes()
}
