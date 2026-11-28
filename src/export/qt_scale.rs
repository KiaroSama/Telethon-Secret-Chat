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
fn axis<T>(
    source: (usize, usize),
    taps: &Taps,
    horizontal: bool,
    channels: usize,
    sample: impl Fn(usize, usize, usize) -> i64,
    convert: impl Fn(i64) -> T,
) -> Result<Vec<T>> {
    let (sw, sh) = source;
    let (w, h) = if horizontal {
        (taps.len(), sh)
    } else {
        (sw, taps.len())
    };
    let mut result = Vec::new();
    result
        .try_reserve_exact(w * h * channels)
        .map_err(|_| ExportError::Image)?;
    for y in 0..h {
        for x in 0..w {
            let (start, weights) = &taps[if horizontal { x } else { y }];
            for channel in 0..channels {
                let sum = weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| {
                        let value = if horizontal {
                            sample((start + k).min(sw - 1), y, channel)
                        } else {
                            sample(x, (start + k).min(sh - 1), channel)
                        };
                        value * weight
                    })
                    .sum();
                result.push(convert(sum));
            }
        }
    }
    Ok(result)
}

pub fn smooth_scale(
    pixels: &[u8],
    source_width: usize,
    source_height: usize,
    channels: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    scale(
        pixels,
        (source_width, source_height),
        channels,
        (width, height),
        channels,
    )
}

pub(super) fn smooth_scale_rgb(
    pixels: &[u8],
    source: (usize, usize),
    channels: usize,
    target: (usize, usize),
) -> Result<Vec<u8>> {
    scale(pixels, source, channels, target, 3)
}

fn scale(
    pixels: &[u8],
    source: (usize, usize),
    channels: usize,
    target: (usize, usize),
    out_channels: usize,
) -> Result<Vec<u8>> {
    let (sw, sh) = source;
    let (w, h) = target;
    if !matches!(channels, 1..=4)
        || (channels != out_channels && channels != 1)
        || [sw, sh, w, h]
            .into_iter()
            .any(|n| !(1..=10000).contains(&n))
        || sw * sh * channels != pixels.len()
    {
        return Err(ExportError::Image);
    }
    let (x_up, y_up) = (w >= sw, h >= sh);
    let (horizontal, first_shift, second_shift) = match (x_up, y_up) {
        (false, false) => (true, 4, 24),
        (true, false) => (false, 0, 22),
        (false, true) => (true, 0, 22),
        (true, true) => (false, 8, 8),
    };
    let interpolate = |source, target, up| {
        if up {
            up_taps(source, target)
        } else {
            down_taps(source, target)
        }
    };
    let (inter_w, inter_h) = if horizontal { (w, sh) } else { (sw, h) };
    // Widen only one intermediate. Sample input bytes and write final bytes directly.
    let working = (inter_w as u64 * inter_h as u64 * 8 + w as u64 * h as u64) * out_channels as u64
        + pixels.len() as u64
        + ((sw + sh + w + h) * 128) as u64;
    if working > 400 * 1024 * 1024 {
        return Err(ExportError::Image);
    }
    let xt = interpolate(sw, w, x_up)?;
    let yt = interpolate(sh, h, y_up)?;
    let (first, second) = if horizontal { (&xt, &yt) } else { (&yt, &xt) };
    let intermediate = axis(
        source,
        first,
        horizontal,
        out_channels,
        |x, y, c| i64::from(pixels[(y * sw + x) * channels + if channels == 1 { 0 } else { c }]),
        |value| value >> first_shift,
    )?;
    axis(
        (inter_w, inter_h),
        second,
        !horizontal,
        out_channels,
        |x, y, c| intermediate[(y * inter_w + x) * out_channels + c],
        |value| (value >> second_shift) as u8,
    )
}
