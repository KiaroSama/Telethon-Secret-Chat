//! Native Rust `kiaro-secret-chat` library entry point.
//!
//! Wires verified `rust-anything` unit crates into a caller-owned,
//! secret-safe MTProto 2.0 secret-chat surface backed by `Lonami/grammers`.

#![forbid(unsafe_code)]

pub use grammers_client;
pub use grammers_crypto;
pub use grammers_mtsender;
pub use grammers_session;
pub use grammers_tl_types;

pub mod autosave;
pub mod binding_migration;
pub mod custody;
pub mod errors;
pub mod establishment;
pub mod export;
pub mod export_archive;
pub mod ingress;
pub mod lifecycle;
pub mod manager;
pub mod media;
pub mod orchestration;
#[doc(hidden)]
pub mod ported;
pub mod protocol;
pub mod public_api;
pub mod runtime_modes;
pub mod storage_backend;
mod storage_file;
pub mod transport;
