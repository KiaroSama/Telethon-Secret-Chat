//! All transport failures are classified before entering manager logs/events.
use crate::{
    dh::DhConfig,
    error::Result,
    files::FileKey,
    media::{MediaReference, SourceReader},
    protocol::SendEffect,
};
use grammers_tl_types::{self as tl, Serializable};
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
    /// Complete already accepted blocking I/O before releasing manager ownership.
    fn drain_io(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    /// Reopen admission only after prior blocking workers have completed.
    fn resume_io(&self) -> Result<()> {
        Ok(())
    }
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
pub(crate) fn file_handle(file: tl::types::EncryptedFile) -> Vec<u8> {
    tl::enums::InputEncryptedFile::from(tl::types::InputEncryptedFile {
        id: file.id,
        access_hash: file.access_hash,
    })
    .to_bytes()
}
