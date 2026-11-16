//! Client-neutral asynchronous request boundary, implemented by the caller-owned Grammers client.
use super::{error::ExportError, fetch_files::LazyFile, model::FileLocation};
use serde_json::Value;
use std::{future::Future, pin::Pin};
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
#[derive(Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Network, authorization or response-decoding failure without remote diagnostics.
    Transport,
    Rpc {
        code: i32,
        name: String,
        kind: String,
    },
    Local(ExportError),
}
impl std::fmt::Debug for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Export request failed")
    }
}
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Export request failed")
    }
}
impl std::error::Error for FetchError {}
impl From<ExportError> for FetchError {
    fn from(e: ExportError) -> Self {
        Self::Local(e)
    }
}
pub type FetchResult<T> = std::result::Result<T, FetchError>;
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestScope {
    pub takeout: Option<i64>,
    pub range: Option<Value>,
}
pub trait ExportClient: Send + Sync {
    fn request<'a>(
        &'a self,
        _scope: RequestScope,
        _request: Value,
    ) -> BoxFuture<'a, FetchResult<Value>> {
        Box::pin(async { Err(ExportError::Unsupported.into()) })
    }
    fn download<'a>(
        &'a self,
        _location: &'a FileLocation,
        _size: u64,
        _target: &'a mut LazyFile,
    ) -> BoxFuture<'a, FetchResult<()>> {
        Box::pin(async { Err(ExportError::Unsupported.into()) })
    }
}
pub fn rpc_message(error: &FetchError) -> &str {
    match error {
        FetchError::Rpc { name, .. } => name,
        _ => "",
    }
}
