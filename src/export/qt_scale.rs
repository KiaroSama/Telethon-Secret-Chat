//! Exact integer port of Qt 5.15.19 qSmoothScaleImage (Imlib2-derived).
//! Copyright Qt Company / Daniel M. Duley; GPL-3.0 option and retained NOTICE apply.
use super::error::{ExportError, Result};
pub type Taps = Vec<(usize, Vec<i64>)>;
pub fn down_taps(source: usize, target: usize) -> Result<Taps> {
    if source == 0 || target == 0 || source > 10000 || target > 10000 {
        return Err(ExportError::Image);
    }
    let inc = (source as i64 * 65536) / target as i64;
    let cp = ((target as i64 * 16384) + source as i64 - 1) / source as i64;
    if cp == 0 {
        return Err(ExportError::Image);
    }
    let mut taps = Vec::with_capacity(target);
    let mut val = 0i64;
    for _ in 0..target {
        let ap = ((65536 - (val & 65535)) * cp) >> 16;
        let mut weights = vec![ap];
        let mut rest = 16384 - ap;
        while rest > cp {
            weights.push(cp);
            rest -= cp;
        }
        weights.push(rest);
        taps.push(((val >> 16) as usize, weights));
        val += inc;
    }
    Ok(taps)
}
pub fn up_taps(source: usize, target: usize) -> Result<Taps> {
    if source == 0 || target == 0 || source > 10000 || target > 10000 {
        return Err(ExportError::Image);
    }
    let inc = (source as i64 * 65536) / target as i64;
    let mut val = 32768 * source as i64 / target as i64 - 32768;
    let mut taps = Vec::with_capacity(target);
    for _ in 0..target {
        let pos = val >> 16;
        let a = if pos < 0 || pos >= source as i64 - 1 {
            0
        } else {
            (val >> 8) & 255
        };
        taps.push((pos.max(0) as usize, vec![256 - a, a]));
        val += inc;
    }
    Ok(taps)
}
fn horizontal(
    pixels: &[i64],
    width: usize,
    height: usize,
    channels: usize,
    taps: &Taps,
) -> Vec<i64> {
    let mut result = vec![0; taps.len() * height * channels];
    for y in 0..height {
        for (x, (start, weights)) in taps.iter().enumerate() {
            for c in 0..channels {
                let mut value = 0;
                for (k, w) in weights.iter().enumerate() {
                    value += pixels[(y * width + (start + k).min(width - 1)) * channels + c] * w;
                }
                result[(y * taps.len() + x) * channels + c] = value;
            }
        }
    }
    result
}
fn vertical(pixels: &[i64], width: usize, height: usize, channels: usize, taps: &Taps) -> Vec<i64> {
    let mut result = vec![0; taps.len() * width * channels];
    for (y, (start, weights)) in taps.iter().enumerate() {
        for x in 0..width {
            for c in 0..channels {
                let mut value = 0;
                for (k, w) in weights.iter().enumerate() {
                    value += pixels[(((start + k).min(height - 1)) * width + x) * channels + c] * w;
                }
                result[(y * width + x) * channels + c] = value;
            }
        }
    }
    result
}
fn shift(values: &mut [i64], bits: u32) {
    for value in values {
        *value >>= bits;
    }
}
pub fn smooth_scale(
    pixels: &[u8],
    source_width: usize,
    source_height: usize,
    channels: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    if !matches!(channels, 1..=4)
        || source_width == 0
        || source_height == 0
        || width == 0
        || height == 0
        || [source_width, source_height, width, height]
            .iter()
            .any(|n| *n > 10000)
        || pixels.len()
            != source_width
                .checked_mul(source_height)
                .and_then(|n| n.checked_mul(channels))
                .ok_or(ExportError::Image)?
    {
        return Err(ExportError::Image);
    }
    let source = pixels.iter().map(|n| i64::from(*n)).collect::<Vec<_>>();
    let x_up = width >= source_width;
    let y_up = height >= source_height;
    let mut result = match (x_up, y_up) {
        (false, false) => {
            let mut rows = horizontal(
                &source,
                source_width,
                source_height,
                channels,
                &down_taps(source_width, width)?,
            );
            shift(&mut rows, 4);
            let mut result = vertical(
                &rows,
                width,
                source_height,
                channels,
                &down_taps(source_height, height)?,
            );
            shift(&mut result, 24);
            result
        }
        (true, false) => {
            let columns = vertical(
                &source,
                source_width,
                source_height,
                channels,
                &down_taps(source_height, height)?,
            );
            let mut result = horizontal(
                &columns,
                source_width,
                height,
                channels,
                &up_taps(source_width, width)?,
            );
            shift(&mut result, 22);
            result
        }
        (false, true) => {
            let rows = horizontal(
                &source,
                source_width,
                source_height,
                channels,
                &down_taps(source_width, width)?,
            );
            let mut result = vertical(
                &rows,
                width,
                source_height,
                channels,
                &up_taps(source_height, height)?,
            );
            shift(&mut result, 22);
            result
        }
        (true, true) => {
            let mut rows = vertical(
                &source,
                source_width,
                source_height,
                channels,
                &up_taps(source_height, height)?,
            );
            shift(&mut rows, 8);
            let mut result = horizontal(
                &rows,
                source_width,
                height,
                channels,
                &up_taps(source_width, width)?,
            );
            shift(&mut result, 8);
            result
        }
    };
    Ok(result.drain(..).map(|n| n as u8).collect())
}
