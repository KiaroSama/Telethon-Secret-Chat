//! Qt 5.15.19/TDesktop JPEG metadata and thumbnail path, derived from the baseline (GPL-3.0).

use mozjpeg::{ColorSpace, Compress, Decompress};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
};

const DEFAULT_DPM: i64 = 3780;
const ICC_HEADER: &[u8] = b"ICC_PROFILE\0";
const MAX_CHUNK: usize = 65519;

fn segments(data: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut start = 2usize;
    std::iter::from_fn(move || {
        while let Some(marker) = data.get(start..start.checked_add(4)?) {
            if marker[0] != 255 || matches!(marker[1], 0xd9 | 0xda) {
                return None;
            }
            if marker[1] == 255 {
                start += 1;
                continue;
            }
            let length = usize::from(u16::from_be_bytes([marker[2], marker[3]]));
            let end = start.checked_add(2 + length)?;
            let payload = data.get(start + 4..end.min(data.len())).unwrap_or_default();
            start = end;
            return Some((marker[1], payload));
        }
        None
    })
}
fn metadata(data: &[u8]) -> ([i64; 2], Vec<u8>) {
    let mut dpm = [DEFAULT_DPM; 2];
    let mut icc = Vec::new();
    for (kind, part) in segments(data) {
        if kind == 0xe0 && part.starts_with(b"JFIF\0") && part.len() >= 12 {
            for (i, d) in dpm.iter_mut().enumerate() {
                let density = f64::from(u16::from_be_bytes([part[8 + i * 2], part[9 + i * 2]]));
                let value = match part[7] {
                    1 => (100.0 * density / 2.54) as i64,
                    2 => (100.0 * density) as i64,
                    _ => 0,
                };
                if value != 0 {
                    *d = value;
                }
            }
        } else if kind == 0xe2 && part.len() > 146 && part.starts_with(ICC_HEADER) {
            icc.extend_from_slice(&part[14..]);
        }
    }
    (dpm, icc)
}
fn density(x: i64, y: i64) -> [i64; 3] {
    let round = |x: f64| (x + 0.5) as i64;
    let error = |x: f64| (x - round(x) as f64).abs();
    let inch = [x, y].map(|d| d as f64 * 2.54 / 100.0);
    let cm = [x, y].map(|d| d as f64 / 100.0);
    if error(inch[0]) + error(inch[1]) < (error(cm[0]) + error(cm[1])) * 2.54 {
        [1, round(inch[0]), round(inch[1])]
    } else {
        [
            2,
            x.saturating_add(50).div_euclid(100),
            y.saturating_add(50).div_euclid(100),
        ]
    }
}
pub fn source_metadata(data: Vec<u8>) -> Result<Value, String> {
    let ([x, y], icc) = metadata(&data);
    Ok(json!({"$tuple":[{"$tuple":[x,y]},{"$bytes":icc}]}))
}
pub fn density_fields(dpm_x: i64, dpm_y: i64) -> Result<Value, String> {
    Ok(json!({"$tuple":density(dpm_x,dpm_y)}))
}
fn marker(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    let body = &payload[..payload.len().min(65533)];
    out.extend([255, kind]);
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}
fn comments(data: &[u8]) -> Vec<u8> {
    let mut texts = HashMap::new();
    for (kind, part) in segments(data) {
        if kind != 0xfe {
            continue;
        }
        let message = String::from_utf8_lossy(part);
        let (key, value) = message
            .split_once(": ")
            .filter(|(k, _)| !k.contains(' '))
            .unwrap_or(("Description", &message));
        if !key.is_empty() {
            texts.insert(key.to_owned(), value.to_owned());
        }
    }
    let mut result = Vec::new();
    let mut entries: Vec<_> = texts.into_iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    for (k, v) in entries {
        marker(&mut result, 0xfe, format!("{k}: {v}").as_bytes());
    }
    result
}
pub fn source_comments(data: Vec<u8>) -> Result<Vec<u8>, String> {
    Ok(comments(&data))
}
fn splice(encoded: Vec<u8>, [x, y]: [i64; 2], icc: &[u8], comment: &[u8]) -> Option<Vec<u8>> {
    if encoded.get(2..4)? != [255, 224] || encoded.get(6..11)? != b"JFIF\0" {
        return None;
    }
    let end = 4 + usize::from(u16::from_be_bytes(encoded.get(4..6)?.try_into().ok()?));
    if end < 18 {
        return None;
    }
    let mut out = encoded.get(..end)?.to_vec();
    let [u, x, y] = density(x, y);
    out[13] = u8::try_from(u).ok()?;
    out[14..16].copy_from_slice(&u16::try_from(x).ok()?.to_be_bytes());
    out[16..18].copy_from_slice(&u16::try_from(y).ok()?.to_be_bytes());
    out.extend_from_slice(comment);
    let count = u8::try_from(icc.len().div_ceil(MAX_CHUNK)).ok()?;
    for (i, chunk) in icc.chunks(MAX_CHUNK).enumerate() {
        let mut p = ICC_HEADER.to_vec();
        p.extend([u8::try_from(i + 1).ok()?, count]);
        p.extend_from_slice(chunk);
        marker(&mut out, 0xe2, &p);
    }
    out.extend_from_slice(encoded.get(end..)?);
    Some(out)
}

fn decode(source: &[u8]) -> Option<(Vec<u8>, usize, usize, usize)> {
    let input = Decompress::new_mem(source).ok()?;
    let (w, h) = input.size();
    if w == 0 || h == 0 || w.checked_mul(h)? > 100_000_000 {
        return None;
    }
    let gray = input.color_space() == ColorSpace::JCS_GRAYSCALE;
    let cmyk = matches!(
        input.color_space(),
        ColorSpace::JCS_CMYK | ColorSpace::JCS_YCCK
    );
    let mut decoder = input
        .to_colorspace(if gray {
            ColorSpace::JCS_GRAYSCALE
        } else if cmyk {
            ColorSpace::JCS_CMYK
        } else {
            ColorSpace::JCS_RGB
        })
        .ok()?;
    let pixels = decoder.read_scanlines::<u8>().ok()?;
    decoder.finish().ok()?;
    let pixels = if cmyk {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|c| {
                std::array::from_fn::<_, 3, _>(|i| (u16::from(c[i]) * u16::from(c[3]) / 255) as u8)
            })
            .collect()
    } else {
        pixels
    };
    Some((pixels, w, h, if gray { 1 } else { 3 }))
}
fn json_bytes(v: &Value) -> Option<Vec<u8>> {
    v.get("$bytes")?
        .as_array()?
        .iter()
        .map(|n| u8::try_from(n.as_u64()?).ok())
        .collect()
}
fn thumbnail(source: &[u8], width: usize, height: usize, quality: f32) -> Option<Vec<u8>> {
    let (pixels, w, h, channels) = decode(source)?;
    let (dpm, icc) = metadata(source);
    let mut dims = vec![i64::try_from(h).ok()?, i64::try_from(w).ok()?];
    if channels == 3 {
        dims.push(3);
    }
    let converted = super::tdexport_qt_icc::convert_to_srgb(pixels, dims, icc).ok()?;
    let converted = converted.get("$tuple")?.as_array()?;
    let mut samples = json_bytes(converted.first()?)?;
    let icc = json_bytes(converted.get(1)?)?;
    let mut gray = channels == 1;
    if (width, height) != (w, h) {
        let rows = samples
            .chunks_exact(w * channels)
            .map(|row| {
                row.chunks_exact(channels)
                    .map(|p| {
                        if gray {
                            vec![i64::from(p[0]); 3]
                        } else {
                            p.iter().map(|&c| i64::from(c)).collect()
                        }
                    })
                    .collect()
            })
            .collect();
        samples = super::tdexport_qt_scale::smooth_scale(
            rows,
            i64::try_from(width).ok()?,
            i64::try_from(height).ok()?,
        )
        .ok()?
        .into_iter()
        .flatten()
        .flatten()
        .map(|v| v as u8)
        .collect();
        gray = false;
    }
    let mut encoder = Compress::new(if gray {
        ColorSpace::JCS_GRAYSCALE
    } else {
        ColorSpace::JCS_RGB
    });
    encoder.set_size(width, height);
    encoder.set_quality(quality);
    encoder.set_chroma_sampling_pixel_sizes((1, 1), (1, 1));
    let mut encoder = encoder.start_compress(Vec::new()).ok()?;
    encoder.write_scanlines(&samples).ok()?;
    splice(
        encoder.finish().ok()?,
        dpm,
        if gray { &[] } else { &icc },
        &comments(source),
    )
}

pub fn desktop_thumb(
    source: Vec<u8>,
    width: i64,
    height: i64,
    quality: Option<i64>,
) -> Result<Option<Vec<u8>>, String> {
    if width <= 0
        || height <= 0
        || width > 65500
        || height > 65500
        || width.saturating_mul(height) > 100_000_000
    {
        return Ok(None);
    }
    let quality = quality.filter(|&q| q >= 0).map_or(75, |q| q.min(100));
    // The safe codec wrapper contains native library failures by unwinding; the baseline returns None.
    Ok(catch_unwind(AssertUnwindSafe(|| {
        thumbnail(&source, width as usize, height as usize, quality as f32)
    }))
    .ok()
    .flatten())
}

#[test]
fn adobe_thumbnail_matches_the_full_jpeg_fixture() {
    let src = include_bytes!("../../tests/fixtures/tdexport_icc/adobe-thumbnail.source.jpg");
    let want = include_bytes!("../../tests/fixtures/tdexport_icc/adobe-thumbnail.expected.jpg");
    assert_eq!(
        desktop_thumb(src.to_vec(), 8, 8, None),
        Ok(Some(want.to_vec()))
    );
}
