//! The Rust port of `rust/adapters/tdexport_qt_scale_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use serde_json::{Value, json};

type Image = Vec<Vec<Vec<i64>>>;
type Taps = Vec<(i64, Vec<i64>)>;
const ZERO: &str = "ZeroDivisionError: integer division or modulo by zero";
fn floor_div(n: i128, d: i128) -> i128 {
    n / d - i128::from(n % d != 0 && (n < 0) != (d < 0))
}
fn taps(source: i64, target: i64, up: bool) -> Result<Taps, String> {
    if target == 0 || (!up && source == 0) {
        return Err(ZERO.into());
    }
    if target < 0 {
        return Ok(Vec::new());
    }
    if target > 65_536 {
        return Err("ValueError: scale dimensions exceed the supported bound".into());
    }
    let (s, t) = (i128::from(source), i128::from(target));
    let inc = floor_div(s << 16, t);
    let mut val = if up {
        floor_div(0x8000 * s, t) - 0x8000
    } else {
        0
    };
    let cp = if up {
        0
    } else {
        floor_div((t << 14) + s - 1, s)
    };
    if !up && cp <= 0 {
        return Err("ValueError: invalid scale weights".into());
    }
    let mut out = Vec::new();
    for _ in 0..target {
        let pos = val >> 16;
        let (pos, weights) = if up {
            let a = if pos < 0 || pos >= s - 1 {
                0
            } else {
                ((val >> 8) & 255) as i64
            };
            (pos.max(0), vec![256 - a, a])
        } else {
            let ap = ((0x10000 - (val & 0xffff)) * cp) >> 16;
            let mut rest = (1 << 14) - ap;
            let mut weights = vec![ap as i64];
            while rest > cp {
                weights.push(cp as i64);
                rest -= cp;
            }
            weights.push(rest as i64);
            (pos, weights)
        };
        out.push((
            i64::try_from(pos).map_err(|_| "OverflowError: source index out of range")?,
            weights,
        ));
        val += inc;
    }
    Ok(out)
}
fn tape(source: i64, target: i64, up: bool) -> Result<Vec<Value>, String> {
    Ok(taps(source, target, up)?
        .into_iter()
        .map(|(i, w)| json!({"$tuple":[i,w]}))
        .collect())
}
pub fn down_taps(source: i64, target: i64) -> Result<Vec<Value>, String> {
    tape(source, target, false)
}
pub fn up_taps(source: i64, target: i64) -> Result<Vec<Value>, String> {
    tape(source, target, true)
}

fn axis(p: &Image, t: &Taps, horizontal: bool, shift: u32) -> Result<Image, String> {
    if t.is_empty() {
        return Err("ValueError: max() iterable argument is empty".into());
    }
    let width = p[0].len();
    let (h, w) = if horizontal {
        (p.len(), t.len())
    } else {
        (t.len(), width)
    };
    let source_last = if horizontal { width - 1 } else { p.len() - 1 };
    Ok((0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let (start, weights) = &t[if horizontal { x } else { y }];
                    (0..p[0][0].len())
                        .map(|c| {
                            weights
                                .iter()
                                .enumerate()
                                .map(|(i, &a)| {
                                    let at =
                                        (*start + i as i64).clamp(0, source_last as i64) as usize;
                                    (if horizontal { p[y][at][c] } else { p[at][x][c] }) * a
                                })
                                .sum::<i64>()
                                >> shift
                        })
                        .collect()
                })
                .collect()
        })
        .collect())
}

pub fn smooth_scale(pixels: Image, width: i64, height: i64) -> Result<Image, String> {
    let Some(row) = pixels.first() else {
        return Err("ValueError: not enough values to unpack (expected 2, got 1)".into());
    };
    let Some(pixel) = row.first() else {
        return if width == 0 || height == 0 {
            Err(ZERO.into())
        } else {
            Err("IndexError: index -1 is out of bounds for axis 0 with size 0".into())
        };
    };
    if pixels
        .iter()
        .any(|r| r.len() != row.len() || r.iter().any(|p| p.len() != pixel.len()))
    {
        return Err("ValueError: invalid source array shape".into());
    }
    if let Some(n) = pixels
        .iter()
        .flatten()
        .flatten()
        .find(|n| !(0i64..=255).contains(*n))
    {
        return Err(format!(
            "OverflowError: Python integer {n} out of bounds for uint8"
        ));
    }
    let (source_w, source_h) = (row.len() as i64, pixels.len() as i64);
    let (x, y) = (width >= source_w, height >= source_h);
    let xt = taps(source_w, width, x)?;
    let yt = taps(source_h, height, y)?;
    let mut result = match (x, y) {
        (false, false) => axis(&axis(&pixels, &xt, true, 4)?, &yt, false, 24)?,
        (true, false) => axis(&axis(&pixels, &yt, false, 0)?, &xt, true, 22)?,
        (false, true) => axis(&axis(&pixels, &xt, true, 0)?, &yt, false, 22)?,
        (true, true) => axis(&axis(&pixels, &yt, false, 8)?, &xt, true, 8)?,
    };
    result
        .iter_mut()
        .flatten()
        .flatten()
        .for_each(|n| *n &= 255);
    Ok(result)
}

#[test]
fn interpolation_blends_the_middle_pixel() {
    assert_eq!(
        smooth_scale(vec![vec![vec![0], vec![200]]], 4, 1),
        Ok(vec![vec![vec![0], vec![50], vec![150], vec![200]]])
    );
}

#[test]
fn downsample_taps_use_the_qt_fixed_point_weights() {
    assert_eq!(
        down_taps(6, 3).map(|taps| taps[0].clone()),
        Ok(serde_json::json!({"$tuple":[0,[8192,8192]]}))
    );
}
