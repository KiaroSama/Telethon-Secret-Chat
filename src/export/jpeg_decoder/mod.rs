//! Pure Rust JPEG decoder, adapted from image-rs jpeg-decoder 0.3.2.
//! Local changes: libjpeg 13-bit integer IDCT, alternating fancy-upsample
//! rounding, 16-bit RGB conversion, and portable execution on every CPU.
//! Upstream authors and MIT/Apache licenses are preserved alongside this module.
#![forbid(unsafe_code)]

pub use decoder::{ColorTransform, Decoder, PixelFormat};

use std::io;
mod decoder;
mod error;
mod huffman;
mod idct;
mod marker;
mod parser;
mod upsampler;
mod worker;
fn read_u8<R: io::Read>(reader: &mut R) -> io::Result<u8> {
    let mut b = [0];
    reader.read_exact(&mut b)?;
    Ok(b[0])
}
fn read_u16_from_be<R: io::Read>(reader: &mut R) -> io::Result<u16> {
    let mut b = [0; 2];
    reader.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
