//! QJpegHandler metadata and mozjpeg-compatible pure-Rust thumbnail pipeline.
use super::jpeg_decoder;
use super::{
    error::{ExportError, Result},
    qt_icc, qt_scale,
};
use mozjpeg_rs::{Encoder, Subsampling, TrellisConfig, TrellisMode};
use std::{collections::BTreeMap, io::Cursor};
pub const DEFAULT_DOTS_PER_METER: u32 = 3780;
const MAX_ICC_CHUNK: usize = 65533 - 14;
fn segments(bytes: &[u8]) -> Vec<(u8, &[u8])> {
    let mut result = Vec::new();
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 255 {
        let marker = bytes[i + 1];
        if marker == 255 {
            i += 1;
            continue;
        }
        if marker == 217 || marker == 218 {
            break;
        }
        let length = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        if length < 2 || i + 2 + length > bytes.len() {
            break;
        }
        result.push((marker, &bytes[i + 4..i + 2 + length]));
        i += 2 + length;
    }
    result
}
pub fn source_metadata(bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
    let mut dpm = [DEFAULT_DOTS_PER_METER; 2];
    let mut chunks = BTreeMap::new();
    let mut expected_chunks = None;
    let mut invalid_icc = false;
    for (marker, payload) in segments(bytes) {
        if marker == 224 && payload.starts_with(b"JFIF\0") && payload.len() >= 12 {
            let unit = payload[7];
            for axis in 0..2 {
                let density = u16::from_be_bytes([payload[8 + axis * 2], payload[9 + axis * 2]]);
                let value = match unit {
                    1 => (100.0 * f64::from(density) / 2.54) as u32,
                    2 => 100 * u32::from(density),
                    _ => continue,
                };
                if value != 0 {
                    dpm[axis] = value;
                }
            }
        } else if marker == 226 && payload.len() >= 14 && payload.starts_with(b"ICC_PROFILE\0") {
            let (index, count) = (payload[12], payload[13]);
            invalid_icc |= index == 0
                || count == 0
                || index > count
                || expected_chunks.is_some_and(|previous| previous != count)
                || chunks.insert(index, payload[14..].to_vec()).is_some();
            expected_chunks = Some(count);
        }
    }
    let icc = if !invalid_icc
        && expected_chunks.is_some_and(|count| chunks.len() == usize::from(count))
    {
        chunks.into_values().flatten().collect()
    } else {
        Vec::new()
    };
    ((dpm[0], dpm[1]), icc)
}
pub fn density_fields(dpm_x: u32, dpm_y: u32) -> (u8, u32, u32) {
    let round = |v: f64| (v + 0.5) as u32;
    let inch_x = f64::from(dpm_x) * 2.54 / 100.0;
    let inch_y = f64::from(dpm_y) * 2.54 / 100.0;
    let diff_inch =
        (inch_x - f64::from(round(inch_x))).abs() + (inch_y - f64::from(round(inch_y))).abs();
    let diff_cm = ((f64::from(dpm_x) / 100.0 - f64::from(round(f64::from(dpm_x) / 100.0))).abs()
        + (f64::from(dpm_y) / 100.0 - f64::from(round(f64::from(dpm_y) / 100.0))).abs())
        * 2.54;
    if diff_inch < diff_cm {
        (1, round(inch_x), round(inch_y))
    } else {
        (
            2,
            dpm_x.saturating_add(50) / 100,
            dpm_y.saturating_add(50) / 100,
        )
    }
}
pub fn source_comments(bytes: &[u8]) -> Vec<u8> {
    let mut text = BTreeMap::new();
    for (marker, payload) in segments(bytes) {
        if marker != 254 {
            continue;
        }
        // QJpegHandler uses QString::fromUtf8, whose replacement behavior is a
        // JPEG COM-format rule, independent of the export's UTF-8 text I/O.
        let comment = String::from_utf8_lossy(payload);
        let (key, value) = match comment.find(": ") {
            Some(index) if comment.find(' ').is_none_or(|space| space >= index) => {
                (&comment[..index], &comment[index + 2..])
            }
            _ => ("Description", comment.as_ref()),
        };
        if !key.is_empty() {
            text.insert(
                key.encode_utf16().collect::<Vec<_>>(),
                (key.to_owned(), value.to_owned()),
            );
        }
    }
    let mut result = Vec::new();
    for (_, (key, value)) in text {
        let payload = format!("{key}: {value}").into_bytes();
        let payload = &payload[..payload.len().min(65533)];
        result.extend([255, 254]);
        result.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        result.extend_from_slice(payload);
    }
    result
}
pub fn splice(encoded: &[u8], dpm: (u32, u32), icc: &[u8], comments: &[u8]) -> Result<Vec<u8>> {
    if encoded.get(2..4) != Some(&[255, 224]) || encoded.get(6..11) != Some(b"JFIF\0") {
        return Err(ExportError::Image);
    }
    let end = 4 + usize::from(u16::from_be_bytes([encoded[4], encoded[5]]));
    let mut app0 = encoded.get(4..end).ok_or(ExportError::Image)?.to_vec();
    if app0.len() < 14 {
        return Err(ExportError::Image);
    }
    let (unit, x, y) = density_fields(dpm.0, dpm.1);
    let x = u16::try_from(x).map_err(|_| ExportError::Image)?;
    let y = u16::try_from(y).map_err(|_| ExportError::Image)?;
    app0[9] = unit;
    app0[10..12].copy_from_slice(&x.to_be_bytes());
    app0[12..14].copy_from_slice(&y.to_be_bytes());
    let mut result = Vec::with_capacity(encoded.len() + icc.len() + comments.len() + 32);
    result.extend_from_slice(&encoded[..4]);
    result.extend_from_slice(&app0);
    result.extend_from_slice(comments);
    let total = icc.len().div_ceil(MAX_ICC_CHUNK);
    if total > 255 {
        return Err(ExportError::Image);
    }
    for (i, chunk) in icc.chunks(MAX_ICC_CHUNK).enumerate() {
        let length = 14 + chunk.len() + 2;
        result.extend([255, 226]);
        result.extend_from_slice(&(length as u16).to_be_bytes());
        result.extend_from_slice(b"ICC_PROFILE\0");
        result.extend([(i + 1) as u8, total as u8]);
        result.extend_from_slice(chunk);
    }
    result.extend_from_slice(&encoded[end..]);
    Ok(result)
}
/// Decode with pure-Rust integer IDCT, run exact Qt color/scaling arithmetic and
/// encode with the Rust port of mozjpeg's maximum-compression profile.
pub fn desktop_thumb(source: &[u8], size: (u32, u32), quality: Option<u8>) -> Result<Vec<u8>> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(source));
    decoder.set_max_decoding_buffer_size(400 * 1024 * 1024);
    decoder.read_info().map_err(|_| ExportError::Image)?;
    let info = decoder.info().ok_or(ExportError::Image)?;
    if info.width == 0
        || info.height == 0
        || info.width >= 10000
        || info.height >= 10000
        || size.0 == 0
        || size.1 == 0
        || size.0 >= 10000
        || size.1 >= 10000
    {
        return Err(ExportError::Image);
    }
    let decoded = decoder.decode().map_err(|_| ExportError::Image)?;
    drop(decoder); // Release progressive coefficients before ICC/scaling work.
    let (channels, pixels) = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => (1, decoded),
        jpeg_decoder::PixelFormat::RGB24 => (3, decoded),
        jpeg_decoder::PixelFormat::CMYK32 => {
            let rgb = decoded
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let black = u16::from(255 - p[3]);
                    [0, 1, 2].map(|i| ((u16::from(255 - p[i]) * black) / 255) as u8)
                })
                .collect();
            (3, rgb)
        }
        _ => return Err(ExportError::Image),
    };
    let (dpm, original_icc) = source_metadata(source);
    let (pixels, icc) = qt_icc::convert_to_srgb(&pixels, channels, &original_icc);
    let mut pixels = pixels.into_owned();
    let mut channels = channels;
    if size != (u32::from(info.width), u32::from(info.height)) {
        if channels == 1 {
            pixels = pixels.into_iter().flat_map(|p| [p, p, p]).collect();
            channels = 3;
        }
        pixels = qt_scale::smooth_scale(
            &pixels,
            usize::from(info.width),
            usize::from(info.height),
            channels,
            size.0 as usize,
            size.1 as usize,
        )?;
    }
    let encoder = Encoder::max_compression()
        .trellis(TrellisConfig {
            mode: TrellisMode::mozjpeg_exact(),
            ..Default::default()
        })
        .subsampling(Subsampling::S444)
        .force_baseline(true)
        .quality(quality.unwrap_or(75).min(100));
    let encoded = if channels == 1 {
        encoder.encode_gray(&pixels, size.0, size.1)
    } else {
        encoder.encode_rgb(&pixels, size.0, size.1)
    }
    .map_err(|_| ExportError::Image)?;
    splice(
        &encoded,
        dpm,
        if channels == 3 { &icc } else { &[] },
        &source_comments(source),
    )
}
