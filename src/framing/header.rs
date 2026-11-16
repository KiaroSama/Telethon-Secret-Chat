//! Inspect the authenticated wrapper's scalar header before reading its body.
use crate::{
    Identifiable,
    error::{Error, Result},
    types,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub layer: i32,
    pub in_seq_no: i32,
    pub out_seq_no: i32,
    pub body_offset: usize,
}
pub fn inspect_header(bytes: &[u8]) -> Result<Header> {
    if bytes.len() > crate::crypto::MAX_FRAME_LENGTH {
        return Err(Error::WrapperRejected);
    }
    let number = |offset: usize| -> Result<i32> {
        Ok(i32::from_le_bytes(
            bytes
                .get(offset..offset.checked_add(4).ok_or(Error::WrapperRejected)?)
                .ok_or(Error::WrapperRejected)?
                .try_into()
                .map_err(|_| Error::WrapperRejected)?,
        ))
    };
    if number(0)? as u32 != types::DecryptedMessageLayer::CONSTRUCTOR_ID {
        return Err(Error::WrapperRejected);
    }
    let first = *bytes.get(4).ok_or(Error::WrapperRejected)?;
    let (length, prefix) = match first {
        255 => return Err(Error::WrapperRejected),
        254 => {
            let raw = bytes.get(5..8).ok_or(Error::WrapperRejected)?;
            let length =
                usize::from(raw[0]) | (usize::from(raw[1]) << 8) | (usize::from(raw[2]) << 16);
            if length < 254 {
                return Err(Error::WrapperRejected);
            }
            (length, 4usize)
        }
        length => (usize::from(length), 1usize),
    };
    let field = prefix.checked_add(length).ok_or(Error::WrapperRejected)?;
    let fields = 4usize
        .checked_add(field)
        .and_then(|value| value.checked_add((4 - field % 4) % 4))
        .ok_or(Error::WrapperRejected)?;
    let layer = number(fields)?;
    if layer < 46 {
        return Err(Error::WrapperRejected);
    }
    let in_seq_no = number(fields.checked_add(4).ok_or(Error::WrapperRejected)?)?;
    let out_seq_no = number(fields.checked_add(8).ok_or(Error::WrapperRejected)?)?;
    Ok(Header {
        layer,
        in_seq_no,
        out_seq_no,
        body_offset: fields.checked_add(12).ok_or(Error::WrapperRejected)?,
    })
}
