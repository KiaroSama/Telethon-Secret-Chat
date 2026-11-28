//! QJpegHandler metadata and Desktop-compatible JPEG thumbnails.
use super::jpeg_decoder;
use super::{
    error::{ExportError, Result},
    qt_icc, qt_scale,
};
use mozjpeg::{ColorSpace, Compress};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
};
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
/// Optional limits for JPEG thumbnails. The archive keeps the original when a limit is exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedImageBudget {
    /// Maximum estimated bytes for packed decode, color conversion and scaling buffers.
    /// This is not a process memory limit; encoded input and codec bookkeeping are separate.
    pub max_decoded_bytes: usize,
    /// Maximum pixel count in either source or thumbnail.
    pub max_pixels: u64,
    /// Maximum width or height of either image.
    pub max_dimension: u32,
}

impl DecodedImageBudget {
    pub(crate) fn allows_jpeg(self, jpeg: &[u8], target: (u32, u32)) -> bool {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg));
        let Some(info) = decoder.read_info().ok().and_then(|()| decoder.info()) else {
            return false;
        };
        let (sw, sh) = (u32::from(info.width), u32::from(info.height));
        let (tw, th) = target;
        let (source, output) = (u64::from(sw) * u64::from(sh), u64::from(tw) * u64::from(th));
        if [sw, sh, tw, th]
            .into_iter()
            .any(|n| n == 0 || n > self.max_dimension)
            || source.max(output) > self.max_pixels
        {
            return false;
        }
        // Allow two four-byte source surfaces and the RGB scaler's one i64 intermediate.
        let intermediate = if (tw, th) == (sw, sh) {
            0
        } else if tw < sw {
            u64::from(tw) * u64::from(sh)
        } else {
            u64::from(sw) * u64::from(th)
        };
        source
            .checked_mul(8)
            .zip(intermediate.checked_mul(24))
            .zip(output.checked_mul(3))
            .and_then(|((source, intermediate), output)| {
                source.checked_add(intermediate)?.checked_add(output)
            })
            .is_some_and(|bytes| bytes <= self.max_decoded_bytes as u64)
    }
}

/// Decode with integer IDCT, apply Qt color and scaling arithmetic, and encode with mozjpeg.
pub fn desktop_thumb(source: &[u8], size: (u32, u32), quality: Option<u8>) -> Result<Vec<u8>> {
    // The safe mozjpeg wrapper can unwind on a native failure.
    catch_unwind(AssertUnwindSafe(|| thumbnail(source, size, quality)))
        .map_err(|_| ExportError::Image)?
}

fn thumbnail(source: &[u8], size: (u32, u32), quality: Option<u8>) -> Result<Vec<u8>> {
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
    let (converted, icc) = qt_icc::convert_to_srgb(&pixels, channels, &original_icc);
    let mut pixels = match converted {
        Cow::Borrowed(_) => pixels,
        Cow::Owned(converted) => converted,
    };
    let geometry = (usize::from(info.width), usize::from(info.height));
    let target = (size.0 as usize, size.1 as usize);
    let mut gray = channels == 1;
    if geometry != target {
        pixels = qt_scale::smooth_scale_rgb(&pixels, geometry, channels, target)?;
        gray = false;
    }
    let mut encoder = Compress::new(if gray {
        ColorSpace::JCS_GRAYSCALE
    } else {
        ColorSpace::JCS_RGB
    });
    encoder.set_size(target.0, target.1);
    encoder.set_quality(f32::from(quality.unwrap_or(75).min(100)));
    encoder.set_chroma_sampling_pixel_sizes((1, 1), (1, 1));
    let mut encoder = encoder
        .start_compress(Vec::new())
        .map_err(|_| ExportError::Image)?;
    encoder
        .write_scanlines(&pixels)
        .map_err(|_| ExportError::Image)?;
    let encoded = encoder.finish().map_err(|_| ExportError::Image)?;
    splice(
        &encoded,
        dpm,
        if !gray && icc.get(16..20) == Some(&b"RGB "[..]) {
            &icc
        } else {
            &[]
        },
        &source_comments(source),
    )
}
