//! Plaintext saved copies. Protocol deletion does not silently delete an archive.
mod plain;
mod store;
pub use plain::plain_object;
pub use store::{AutoSave, PendingFile, safe_file_name};
