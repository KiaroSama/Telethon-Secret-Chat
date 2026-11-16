//! Layer negotiation and the mirrored sequence-number transform.
mod header;
use crate::error::{Error, Result};
use crate::{Deserializable, crypto::random_bytes, enums, types};
use grammers_tl_types::deserialize::Cursor;
pub use header::{Header, inspect_header};
pub const INITIAL_REMOTE_LAYER: i32 = 46;
pub const MIN_LAYER: i32 = 73;
pub const MAX_LAYER: i32 = 144;
pub const RANDOM_BYTES_LENGTH: usize = 31;
pub fn outgoing_layer(remote: i32) -> i32 {
    remote.clamp(MIN_LAYER, MAX_LAYER)
}
pub fn raise_remote_layer(stored: i32, announced: i32) -> i32 {
    stored.max(announced)
}
pub fn transform_in_seq_no(raw: i32, outbound: bool) -> Result<i32> {
    if raw < 0 {
        return Err(Error::Sequence);
    }
    raw.checked_mul(2)
        .and_then(|n| n.checked_add(i32::from(!outbound)))
        .ok_or(Error::Sequence)
}
pub fn transform_out_seq_no(raw: i32, outbound: bool) -> Result<i32> {
    if raw < 0 {
        return Err(Error::Sequence);
    }
    raw.checked_mul(2)
        .and_then(|n| n.checked_add(i32::from(outbound)))
        .ok_or(Error::Sequence)
}
pub fn wrap(
    message: enums::DecryptedMessage,
    layer: i32,
    in_seq_no: i32,
    out_seq_no: i32,
) -> Result<enums::DecryptedMessageLayer> {
    if layer < 46 || in_seq_no < 0 || out_seq_no < 0 {
        return Err(Error::WrapperRejected);
    }
    Ok(types::DecryptedMessageLayer {
        random_bytes: random_bytes::<31>()?.to_vec(),
        layer,
        in_seq_no,
        out_seq_no,
        message,
    }
    .into())
}
pub fn unwrap(body: &[u8]) -> Result<types::DecryptedMessageLayer> {
    if body.len() > crate::crypto::MAX_FRAME_LENGTH {
        return Err(Error::WrapperRejected);
    }
    inspect_header(body)?;
    crate::wire::validate(body, "DecryptedMessageLayer")
        .map_err(|_| Error::InnerMessageRejected)?;
    let mut cursor = Cursor::from_slice(body);
    let enums::DecryptedMessageLayer::Layer(wrapper) =
        enums::DecryptedMessageLayer::deserialize(&mut cursor)
            .map_err(|_| Error::InnerMessageRejected)?;
    if cursor.pos() != body.len() || wrapper.layer < 46 {
        return Err(Error::WrapperRejected);
    }
    Ok(wrapper)
}
/// Refuse an explicit peer announcement that would require MTProto 1.0.
pub fn require_supported_layer(peer_layer: i32) -> Result<()> {
    if peer_layer < MIN_LAYER {
        Err(Error::WrapperRejected)
    } else {
        Ok(())
    }
}
pub fn raw_seq_no(wire: i32) -> Result<i32> {
    if wire < 0 {
        Err(Error::Sequence)
    } else {
        Ok(wire / 2)
    }
}
