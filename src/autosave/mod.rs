//! Plaintext saved copies. Protocol deletion does not silently delete an archive.
mod plain;
mod store;
mod store_v4;

pub use plain::plain_object;
pub(crate) use store::ArchiveWorker;
pub use store::{AutoSave, PendingFile, safe_file_name};
pub use store_v4::*;
