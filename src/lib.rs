//! Native Rust `kiaro-secret-chat` library entry point.
//!
//! Wires caller-owned MTProto 2.0 secret chats backed by `Lonami/grammers`
//! (Layer 234), including generated end-to-end TL types, durable
//! pre-ack ingress, bounded/transient/strong managers, encrypted media,
//! auto-save, Desktop-compatible rich archive export, and offline binding
//! migration.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as kiaro_secret_chat;
#[cfg(test)]
mod test_support;

pub use grammers_client;
pub use grammers_crypto;
pub use grammers_mtsender;
pub use grammers_session;
pub use grammers_tl_types;
pub use grammers_tl_types::{Deserializable, Identifiable, RawVec, Serializable};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod actions;
pub(crate) mod aes_ige;
pub mod autosave;
pub mod binding;
pub mod binding_migration;
pub mod chat;
pub mod crypto;
pub mod custody;
pub mod dh;
pub mod entities;
pub mod error;
pub mod errors;
pub mod establishment;
pub mod export;
pub mod export_archive;
pub mod files;
pub mod framing;
pub mod ingress;
pub mod lifecycle;
pub mod manager;
pub mod media;
pub mod ogg;
pub mod orchestration;
#[doc(hidden)]
pub mod ported;
pub mod protocol;
pub mod public_api;
pub mod rekey;
pub mod runtime_modes;
pub mod schema_generator;
mod secret_record;
pub mod sequence;
pub mod storage;
pub mod storage_backend;
mod storage_file;
pub mod temporary;
pub mod transport;
pub mod visualization;
pub mod wire;

pub mod deserialize {
    pub use grammers_tl_types::deserialize::*;
    pub type Buffer<'a, 'b> = &'a mut Cursor<'b>;
}

mod generated_common {
    include!(concat!(env!("OUT_DIR"), "/secret_common.rs"));
}

pub use generated_common::{LAYER, name_for_id};

#[allow(clippy::all)]
pub mod types {
    include!(concat!(env!("OUT_DIR"), "/secret_types.rs"));
}

#[allow(clippy::all)]
pub mod enums {
    include!(concat!(env!("OUT_DIR"), "/secret_enums.rs"));
}
