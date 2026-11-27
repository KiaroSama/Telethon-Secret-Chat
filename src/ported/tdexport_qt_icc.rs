//! The Rust port of `rust/adapters/tdexport_qt_icc_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

type Color = [f32; 3];
type Matrix = [Color; 3];
type Points = [(f64, f64); 4];
type Fun = [f32; 7];
const EPS: f32 = 1.0 / 2048.0;
const D50: (f64, f64) = (0.34567, 0.35850);
const D65: (f64, f64) = (0.31271, 0.32902);
const PRIMARIES: [Points; 4] = [
    [(0.64, 0.33), (0.3, 0.6), (0.15, 0.06), D65],
    [(0.64, 0.33), (0.21, 0.71), (0.15, 0.06), D65],
    [(0.68, 0.32), (0.265, 0.69), (0.15, 0.06), D65],
    [(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001), D50],
];
const KNOWN: MatrixSet = [
    [
        [0.436_021_75, 0.222_475_11, 0.013_928_114],
        [0.385_108_8, 0.716_906_7, 0.097_101_54],
        [0.143_081_25, 0.060_618_2, 0.714_158_6],
    ],
    [
        [0.609_718_9, 0.311_102_15, 0.019_476_633],
        [0.205_268_22, 0.625_677_05, 0.060_889_15],
        [0.149_224_74, 0.063_220_96, 0.744_822_44],
    ],
    [
        [0.515_097_3, 0.241_179_56, -0.001_049_103_4],
        [0.291_969_66, 0.692_244_1, 0.041_883_033],
        [0.157_144_92, 0.066_576_45, 0.784_354_27],
    ],
    [
        [0.797_667_26, 0.288_037_45, 0.0],
        [0.135_192_25, 0.711_877, 0.0],
        [0.031_352_53, 0.000_085_662_7, 0.825_188_34],
    ],
];
type MatrixSet = [Matrix; 4];
const ID: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const SRGB_PROFILE: &[u8] = include_bytes!("../../data/srgb-profile.icc");

#[derive(PartialEq)]
enum Curve {
    Table(Vec<u16>),
    Function(Fun),
}

fn map(m: Matrix, v: Color) -> Color {
    std::array::from_fn(|i| (v[0] * m[0][i] + v[1] * m[1][i]) + v[2] * m[2][i])
}
fn mul(l: Matrix, r: Matrix) -> Matrix {
    r.map(|v| map(l, v))
}
fn det([r, g, b]: Matrix) -> f32 {
    (r[0] * (b[2] * g[1] - g[2] * b[1]) - r[1] * (b[2] * g[0] - g[2] * b[0]))
        + r[2] * (b[1] * g[0] - g[1] * b[0])
}
fn inverse(m: Matrix) -> Matrix {
    let [r, g, b] = m;
    let inv = [
        [
            g[1] * b[2] - b[1] * g[2],
            b[1] * r[2] - r[1] * b[2],
            r[1] * g[2] - g[1] * r[2],
        ],
        [
            b[0] * g[2] - g[0] * b[2],
            r[0] * b[2] - b[0] * r[2],
            g[0] * r[2] - r[0] * g[2],
        ],
        [
            g[0] * b[1] - b[0] * g[1],
            b[0] * r[1] - r[0] * b[1],
            r[0] * g[1] - g[0] * r[1],
        ],
    ];
    let factor = 1.0 / det(m);
    inv.map(|v| v.map(|x| x * factor))
}
fn xyz((x, y): (f64, f64)) -> Color {
    [(x / y) as f32, 1.0, ((1.0 - x - y) / y) as f32]
}
fn near<const N: usize>(a: [f32; N], b: [f32; N], limit: f32, inclusive: bool) -> bool {
    a.into_iter().zip(b).all(|(x, y)| {
        if inclusive {
            (x - y).abs() <= limit
        } else {
            (x - y).abs() < limit
        }
    })
}
fn near_matrix(a: Matrix, b: Matrix) -> bool {
    a.into_iter().zip(b).all(|(x, y)| near(x, y, EPS, false))
}
fn diag(v: Color) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| if i == j { v[i] } else { 0.0 }))
}
fn matrix(points: Points) -> Matrix {
    let [r, g, b, w] = points.map(xyz);
    let d50 = xyz(D50);
    let m = [r, g, b];
    let m = mul(m, diag(map(inverse(m), w)));
    if near(w, d50, EPS, false) {
        return m;
    }
    let brad = [
        [0.8951, -0.7502, 0.0389],
        [0.2664, 1.7135, -0.0685],
        [-0.1614, 0.0367, 1.0296],
    ];
    let inv = [
        [0.986_992_9, 0.432_305_3, -0.008_528_7],
        [-0.147_054_3, 0.518_360_3, 0.040_042_8],
        [0.159_962_7, 0.049_291_2, 0.968_486_7],
    ];
    let x = map(brad, d50);
    let y = map(brad, w);
    let adaptation = mul(inv, mul(diag(std::array::from_fn(|i| x[i] / y[i])), brad));
    mul(adaptation, m)
}
fn srgb() -> Fun {
    [
        1.0 / 1.055,
        0.055 / 1.055,
        1.0 / 12.92,
        0.04045,
        0.0,
        0.0,
        2.4,
    ]
}
fn gamma(g: f32) -> Fun {
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, g]
}
fn match_fun(a: Fun, b: Fun) -> bool {
    near(a, b, 1.0 / 512.0, true)
}
fn same(a: &Curve, b: &Curve) -> bool {
    match (a, b) {
        (Curve::Function(a), Curve::Function(b)) => match_fun(*a, *b),
        _ => a == b,
    }
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}
fn be32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(bytes(b, at)?) as usize)
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes(b, at)?))
}
fn fix(b: &[u8], at: usize) -> Option<f32> {
    Some(i32::from_be_bytes(bytes(b, at)?) as f32 / 65536.0)
}
fn color(b: &[u8], at: usize) -> Option<Color> {
    Some([fix(b, at)?, fix(b, at + 4)?, fix(b, at + 8)?])
}
fn curve(b: &[u8]) -> Option<Curve> {
    if b.starts_with(b"curv") {
        let len = be32(b, 8)?;
        if len > 65_536 || b.len() < 12 + 2 * len {
            return None;
        }
        let linear = [1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        if len == 0 {
            return Some(Curve::Function(linear));
        }
        if len == 1 {
            return Some(Curve::Function(gamma(f32::from(be16(b, 12)?) / 256.0)));
        }
        let values: Vec<_> = (0..len)
            .map(|i| be16(b, 12 + i * 2))
            .collect::<Option<_>>()?;
        if values.windows(2).any(|x| x[1] < x[0]) {
            return None;
        }
        if values[0] == 0 && values[len - 1] == u16::MAX {
            if len == 2 {
                return Some(Curve::Function(linear));
            }
            let test = match len {
                26 => Some(([6, 12, 18], [3062, 12824, 31237])),
                1024 => Some(([257, 513, 768], [3366, 14116, 34318])),
                4096 => Some(([515, 1025, 2051], [960, 3342, 14079])),
                _ => None,
            };
            if let Some((at, expected)) = test
                && at.map(|i| values[i]) == expected
            {
                return Some(Curve::Function(srgb()));
            }
        }
        return Some(Curve::Table(values));
    }
    if !b.starts_with(b"para") || b.len() < 16 {
        return None;
    }
    let kind = usize::from(be16(b, 8)?);
    let count = *[1, 3, 4, 5, 7].get(kind)?;
    let mut p = [0.0; 7];
    for (i, v) in p.iter_mut().take(count).enumerate() {
        *v = fix(b, 12 + 4 * i)?;
    }
    let [g, a, b, c, d, e, f] = p;
    Some(Curve::Function(match kind {
        0 => gamma(g),
        1 | 2 if a != 0.0 => [a, b, 0.0, -b / a, c, c, g],
        1 | 2 => return None,
        _ => [a, b, c, d, e, f, g],
    }))
}
fn parse(profile: &[u8]) -> Option<(Matrix, [Curve; 3])> {
    if profile.len() < 132 || &profile[36..40] != b"acsp" || &profile[20..24] != b"XYZ " {
        return None;
    }
    let gray = &profile[16..20] == b"GRAY";
    let class = &profile[12..16];
    if !(gray || &profile[16..20] == b"RGB ")
        || !(class == b"mntr" || class == b"scnr" || gray && class == b"prtr")
    {
        return None;
    }
    let size = be32(profile, 0)?;
    let count = be32(profile, 128)?;
    let edge = 132usize.checked_add(count.checked_mul(12)?)?;
    if size > profile.len() || edge > size || !near(color(profile, 68)?, xyz(D50), EPS, false) {
        return None;
    }
    let mut tags = HashMap::new();
    for at in (132..edge).step_by(12) {
        let start = be32(profile, at + 4)?;
        let count = be32(profile, at + 8)?;
        let end = start.checked_add(count)?;
        if start < edge || start % 4 != 0 || count < 12 || end > size {
            return None;
        }
        tags.insert(bytes::<4>(profile, at)?, &profile[start..end]);
    }
    let point = |key: &[u8; 4]| {
        let tag = tags.get(key)?;
        (tag.len() >= 20 && tag.starts_with(b"XYZ ")).then_some(())?;
        color(tag, 8)
    };
    let white = point(b"wtpt")?;
    let (m, keys) = if gray {
        let den = (1.0 + white[2]) + white[0];
        if (white[1] - 1.0).abs() > 1e-5 || den == 0.0 {
            return None;
        }
        let m = if near(white, xyz(D65), EPS, false) {
            matrix(PRIMARIES[0])
        } else {
            let y = 1.0 / den;
            let x = white[0] * y;
            if !(0.0..=1.0).contains(&x)
                || !(0.0 < y && y <= 1.0)
                || f64::from(x) + f64::from(y) > 1.0
            {
                return None;
            }
            let mut points = PRIMARIES[0];
            points[3] = (f64::from(x), f64::from(y));
            matrix(points)
        };
        (m, [b"kTRC"; 3])
    } else {
        let m = [point(b"rXYZ")?, point(b"gXYZ")?, point(b"bXYZ")?];
        let m = KNOWN
            .iter()
            .position(|&known| near_matrix(m, known))
            .map_or(m, |i| matrix(PRIMARIES[i]));
        let rgb = [b"rTRC", b"gTRC", b"bTRC"];
        if rgb.iter().any(|&k| !tags.contains_key(k)) {
            return None;
        }
        let apple = [b"aarg", b"aagg", b"aabg"];
        (
            m,
            if apple.iter().all(|&k| tags.contains_key(k)) {
                apple
            } else {
                rgb
            },
        )
    };
    let [r, g, b] = keys.map(|k| curve(tags.get(k)?));
    let mut out = [r?, g?, b?];
    if det(m).abs() <= 1e-5 {
        return None;
    }
    if let [Curve::Function(f), Curve::Function(g), Curve::Function(b)] = out
        && match_fun(f, g)
        && match_fun(f, b)
    {
        let f = if near(
            [f[0], f[1], f[3], f[4]],
            [1.0, 0.0, 0.0, 0.0],
            1.0 / 512.0,
            true,
        ) {
            gamma(if (f[6] - 1.0).abs() <= 1e-5 {
                1.0
            } else {
                f[6]
            })
        } else if match_fun(f, srgb()) {
            srgb()
        } else {
            f
        };
        out = std::array::from_fn(|_| Curve::Function(f));
    }
    Some((m, out))
}
fn apply([a, b, c, d, e, f, g]: Fun, x: f32) -> f32 {
    if x < d {
        c * x + f
    } else {
        (f64::from(a * x + b).powf(f64::from(g)) as f32) + e
    }
}
fn lut(curve: &Curve) -> Option<[u16; 256]> {
    let mut out = [0; 256];
    for (i, v) in out.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        *v = match curve {
            Curve::Table(values) => {
                let last = values.len() - 1;
                let at = x * last as f32;
                let lo = at.floor() as usize;
                let hi = (lo + 1).min(last);
                let f = at - lo as f32;
                let y = (f32::from(values[lo]) * (1.0 - f) + f32::from(values[hi]) * f)
                    * (1.0 / 65535.0);
                (y * 65280.0 + 0.5).floor().clamp(0.0, 65280.0) as u16
            }
            Curve::Function(fun) => {
                let y = apply(*fun, x);
                if !y.is_finite() {
                    return None;
                }
                let scaled = y * 65280.0;
                let rounded = if scaled < 0.0 {
                    let base = (scaled - 1.0).trunc();
                    (scaled - base + 0.5).trunc() + base
                } else {
                    (scaled + 0.5).trunc()
                };
                rounded.rem_euclid(65536.0) as u16
            }
        };
    }
    Some(out)
}
static TARGET_LUT: LazyLock<[u16; 4081]> = LazyLock::new(|| {
    let [a, b, c, d, e, f, g] = srgb();
    let ia = f64::from(1.0 / a).powf(f64::from(g)) as f32;
    let inverse = [ia, -ia * e, 1.0 / c, c * d + f, -b / a, -f / c, 1.0 / g];
    std::array::from_fn(|i| {
        (apply(inverse, (i as f64 / 4080.0) as f32) * 65280.0 + 0.5).floor() as u16
    })
});
fn outcome(pixels: &[u8], profile: &[u8]) -> Value {
    json!({"$tuple":[{"$bytes":pixels},{"$bytes":profile}]})
}

pub fn convert_to_srgb(
    pixels: Vec<u8>,
    shape: Vec<i64>,
    profile: Vec<u8>,
) -> Result<Value, String> {
    let (pixels, profile) = convert_to_srgb_bytes(pixels, shape, profile)?;
    Ok(outcome(&pixels, &profile))
}

// Preserve the public parity wrapper while allowing image processing to retain
// packed byte buffers instead of allocating one JSON value for every sample.
pub(crate) fn convert_to_srgb_bytes(
    pixels: Vec<u8>,
    shape: Vec<i64>,
    profile: Vec<u8>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let dims = shape
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let known = shape
        .iter()
        .filter(|&&d| d >= 0)
        .try_fold(1usize, |a, &d| a.checked_mul(d as usize));
    let unknown = shape.iter().filter(|&&d| d < 0).count();
    if unknown > 1 {
        return Err("ValueError: can only specify one unknown dimension".into());
    }
    let fits = if unknown == 1 {
        known.is_some_and(|k| k != 0 && pixels.len().is_multiple_of(k))
    } else {
        known == Some(pixels.len())
    };
    if !fits {
        return Err(format!(
            "ValueError: cannot reshape array of size {} into shape ({dims})",
            pixels.len()
        ));
    }
    let Some((m, curves)) = parse(&profile) else {
        return Ok((pixels, profile));
    };
    let target = matrix(PRIMARIES[0]);
    if near_matrix(m, target)
        && curves
            .iter()
            .all(|c| matches!(c,Curve::Function(f) if match_fun(*f,srgb())))
    {
        return Ok((pixels, profile));
    }
    let gray = shape.len() == 2;
    if !gray && (shape.len() != 3 || shape[2] != 3) {
        return Err("ValueError: ICC conversion requires a uint8 RGB or grayscale array".into());
    }
    let transform = mul(inverse(target), m);
    if det(transform).abs() <= 1e-5 {
        return Ok((pixels, profile));
    }
    let Some(a) = lut(&curves[0]) else {
        return Ok((pixels, profile));
    };
    let Some(g) = lut(&curves[1]) else {
        return Ok((pixels, profile));
    };
    let Some(b) = lut(&curves[2]) else {
        return Ok((pixels, profile));
    };
    let tables = if same(&curves[0], &curves[1]) && same(&curves[0], &curves[2]) {
        [a; 3]
    } else {
        [a, g, b]
    };
    let use_m = !near_matrix(transform, ID);
    let mut result = Vec::with_capacity(pixels.len());
    for pixel in pixels.chunks_exact(if gray { 1 } else { 3 }) {
        let mut lin: Color = std::array::from_fn(|i| {
            f32::from(tables[i][usize::from(pixel[if gray { 0 } else { i }])]) * (1.0 / 65280.0)
        });
        if use_m {
            lin = map(transform, lin).map(|v| v.clamp(0.0, 1.0));
        }
        let ix = lin.map(|x| (x * 4080.0).round_ties_even());
        if ix.iter().any(|x| !(0.0..=4080.0).contains(x)) {
            return Ok((pixels, profile));
        }
        let [r, g, b] = ix.map(|i| (u32::from(TARGET_LUT[i as usize]) + 128) >> 8);
        if gray {
            result.push(((r * 11 + g * 16 + b * 5) >> 5) as u8);
        } else {
            result.extend([r as u8, g as u8, b as u8]);
        }
    }
    Ok((result, SRGB_PROFILE.to_vec()))
}

#[test]
fn invalid_profile_preserves_original_pixels_and_profile() {
    assert_eq!(
        convert_to_srgb(vec![255], vec![1, 1], vec![0]),
        Ok(serde_json::json!({"$tuple":[{"$bytes":[255]},{"$bytes":[0]}]}))
    );
}
