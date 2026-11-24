//! Secret-chat protocol components for Telegram's MTProto 2.0.
//!
//! The caller owns the grammers client, its authorization and the update stream.
#![forbid(unsafe_code)]
#[cfg(test)]
extern crate self as kiaro_secret_chat;
#[cfg(test)]
mod test_support;

// One canonical SDK type graph, including the pre-ack custody supplement.
pub use grammers_client;
pub use grammers_mtsender;
pub use grammers_session;
pub use grammers_tl_types;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod actions;
pub(crate) mod aes_ige;
pub mod autosave;
pub mod binding;
pub mod chat;
pub mod crypto;
pub mod custody;
pub mod dh;
pub mod entities;
pub mod error;
pub mod export;
pub mod files;
pub mod framing;
pub mod ingress;
pub mod manager;
pub mod media;
pub mod ogg;
pub mod protocol;
pub mod rekey;
pub mod schema_generator;
mod secret_record;
pub mod sequence;
pub mod storage;
pub(crate) mod temporary;
pub mod transport;
pub mod visualization;
pub mod wire;

// The grammers generator targets crate-level traits and modules.
pub use grammers_tl_types::{Deserializable, Identifiable, RawVec, Serializable};
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
