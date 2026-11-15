//! Secret-chat protocol components for Telegram's MTProto 2.0.
//!
//! The caller owns the grammers client, its authorization and the update stream.
#![forbid(unsafe_code)]

pub mod chat;
pub mod crypto;
pub mod dh;
pub mod entities;
pub mod error;
pub mod files;
pub mod framing;
pub mod ogg;
pub mod protocol;
pub mod rekey;
pub mod sequence;
pub mod storage;
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
