//! Exact float32 matrix/TRC path from Qt 5.15.19, without a Qt or native codec runtime.
//! Copyright (C) 2016, 2018 The Qt Company Ltd. GPL-3.0 option; see NOTICE.
use std::{borrow::Cow, collections::BTreeMap};
type Matrix = [[f32; 3]; 3];
#[derive(Clone)]
enum Curve {
    Function([f32; 7]),
    Table(Vec<f32>),
}
fn map(m: Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (v[0] * m[0][i] + v[1] * m[1][i]) + v[2] * m[2][i])
}
fn multiply(left: Matrix, right: Matrix) -> Matrix {
    right.map(|row| map(left, row))
}
fn det(m: Matrix) -> f32 {
    let [r, g, b] = m;
    (r[0] * (b[2] * g[1] - g[2] * b[1]) - r[1] * (b[2] * g[0] - g[2] * b[0]))
        + r[2] * (b[1] * g[0] - g[1] * b[0])
}
fn inverse(m: Matrix) -> Matrix {
    let [r, g, b] = m;
    let scale = 1.0 / det(m);
    [
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
    ]
    .map(|row| row.map(|v| v * scale))
}
fn xyz(p: (f64, f64)) -> [f32; 3] {
    let (x, y) = p;
    [(x / y) as f32, 1.0, ((1.0 - x - y) / y) as f32]
}
fn diagonal(v: [f32; 3]) -> Matrix {
    [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]]
}
fn matrix(points: [(f64, f64); 4]) -> Matrix {
    let white = xyz(points[3]);
    let d50 = xyz((0.34567, 0.35850));
    let mut m = [xyz(points[0]), xyz(points[1]), xyz(points[2])];
    m = multiply(m, diagonal(map(inverse(m), white)));
    if !white
        .iter()
        .zip(d50)
        .all(|(a, b)| (*a - b).abs() < 1.0 / 2048.0)
    {
        let brad = [
            [0.8951, -0.7502, 0.0389],
            [0.2664, 1.7135, -0.0685],
            [-0.1614, 0.0367, 1.0296],
        ];
        let inv = [
            [0.9869929, 0.4323053, -0.0085287],
            [-0.1470543, 0.5183603, 0.0400428],
            [0.1599627, 0.0492912, 0.9684867],
        ];
        let a = map(brad, d50);
        let b = map(brad, white);
        let adaptation = multiply(
            inv,
            multiply(diagonal(std::array::from_fn(|i| a[i] / b[i])), brad),
        );
        m = multiply(adaptation, m);
    }
    m
}
fn srgb() -> [f32; 7] {
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
fn gamma(g: f32) -> [f32; 7] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, g]
}
fn matches(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1.0 / 512.0)
}
fn same(a: &Curve, b: &Curve) -> bool {
    match (a, b) {
        (Curve::Function(a), Curve::Function(b)) => matches(a, b),
        (Curve::Table(a), Curve::Table(b)) => a == b,
        _ => false,
    }
}
fn be16(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        b.get(p..p.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn be32(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        b.get(p..p.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn fixed(b: &[u8], p: usize) -> Option<f32> {
    Some((be32(b, p)? as i32) as f32 / 65536.0)
}
fn curve(tag: &[u8]) -> Option<Curve> {
    if tag.get(..4)? == b"curv" {
        let count = be32(tag, 8)? as usize;
        if count > 65536 || tag.len() < 12 + 2 * count {
            return None;
        }
        if count == 0 {
            return Some(Curve::Function([1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]));
        }
        if count == 1 {
            return Some(Curve::Function(gamma(f32::from(be16(tag, 12)?) / 256.0)));
        }
        let values = (0..count)
            .map(|i| be16(tag, 12 + 2 * i).map(f32::from))
            .collect::<Option<Vec<_>>>()?;
        if values.windows(2).any(|p| p[1] < p[0]) {
            return None;
        }
        if values[0] == 0.0 && values[count - 1] == 65535.0 {
            if count == 2 {
                return Some(Curve::Function([1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]));
            }
            let heuristic = match count {
                26 => Some(([6, 12, 18], [3062.0, 12824.0, 31237.0])),
                1024 => Some(([257, 513, 768], [3366.0, 14116.0, 34318.0])),
                4096 => Some(([515, 1025, 2051], [960.0, 3342.0, 14079.0])),
                _ => None,
            };
            if heuristic
                .is_some_and(|(indices, v)| indices.into_iter().zip(v).all(|(i, x)| values[i] == x))
            {
                return Some(Curve::Function(srgb()));
            }
        }
        return Some(Curve::Table(values));
    }
    if tag.get(..4)? != b"para" || tag.len() < 16 {
        return None;
    }
    let kind = usize::from(be16(tag, 8)?);
    let count = *[1, 3, 4, 5, 7].get(kind)?;
    if tag.len() < 12 + 4 * count {
        return None;
    }
    let p = (0..count)
        .map(|i| fixed(tag, 12 + 4 * i))
        .collect::<Option<Vec<_>>>()?;
    if kind == 0 {
        return Some(Curve::Function(gamma(p[0])));
    }
    let (g, a, b) = (p[0], p[1], p[2]);
    if kind == 1 || kind == 2 {
        if a == 0.0 {
            return None;
        }
        let c = if kind == 2 { p[3] } else { 0.0 };
        return Some(Curve::Function([a, b, 0.0, -b / a, c, c, g]));
    }
    let (c, d) = (p[3], p[4]);
    let (e, f) = if kind == 4 { (p[5], p[6]) } else { (0.0, 0.0) };
    Some(Curve::Function([a, b, c, d, e, f, g]))
}
const PRIMARIES: [[(f64, f64); 4]; 4] = [
    [(0.64, 0.33), (0.3, 0.6), (0.15, 0.06), (0.31271, 0.32902)],
    [(0.64, 0.33), (0.21, 0.71), (0.15, 0.06), (0.31271, 0.32902)],
    [
        (0.68, 0.32),
        (0.265, 0.69),
        (0.15, 0.06),
        (0.31271, 0.32902),
    ],
    [
        (0.7347, 0.2653),
        (0.1596, 0.8404),
        (0.0366, 0.0001),
        (0.34567, 0.3585),
    ],
];
// Qt source decimal spellings deliberately retained; f32 rounding is part of the oracle.
#[allow(clippy::excessive_precision)]
const RECOGNIZED: [Matrix; 4] = [
    [
        [0.4360217452, 0.2224751115, 0.0139281144],
        [0.3851087987, 0.7169067264, 0.0971015394],
        [0.1430812478, 0.0606181994, 0.7141585946],
    ],
    [
        [0.6097189188, 0.3111021519, 0.0194766335],
        [0.2052682191, 0.6256770492, 0.0608891509],
        [0.1492247432, 0.0632209629, 0.7448224425],
    ],
    [
        [0.5150973201, 0.2411795557, -0.0010491034],
        [0.2919696569, 0.6922441125, 0.0418830328],
        [0.157144919, 0.0665764511, 0.7843542695],
    ],
    [
        [0.7976672649, 0.2880374491, 0.0],
        [0.1351922452, 0.7118769884, 0.0],
        [0.0313525312, 8.56627e-05, 0.8251883388],
    ],
];
const SRGB_PROFILE_HEX: &str = "000001cb00000000024000006d6e74725247422058595a2000000000000000000000000061637370000000000000000000000000000000000000000000000000000000010000f6d6000100000000d32d5174050f0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000097258595a000000f0000000146758595a00000104000000146258595a0000011800000014777470740000012c0000001463707274000001400000000c725452430000014c00000020675452430000014c00000020625452430000014c00000020646573630000016c0000005f58595a200000000000006f9f000038f40000039158595a2000000000000062960000b787000018dc58595a2000000000000024a100000f850000b6d358595a20000000000000f34f00010000000116c274657874000000004e2f4100706172610000000000030000000266660000f2a700000d59000013d000000a5b6465736300000000000000057352474200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
fn parse(profile: &[u8]) -> Option<(Matrix, [Curve; 3])> {
    if profile.len() < 132 || profile.get(36..40)? != b"acsp" || profile.get(20..24)? != b"XYZ " {
        return None;
    }
    let size = be32(profile, 0)? as usize;
    let count = be32(profile, 128)? as usize;
    let gray = profile.get(16..20)? == b"GRAY";
    if !gray && profile.get(16..20)? != b"RGB " {
        return None;
    }
    let class = profile.get(12..16)?;
    if !(class == b"mntr" || class == b"scnr" || (gray && class == b"prtr")) {
        return None;
    }
    let table_end = 132usize.checked_add(count.checked_mul(12)?)?;
    if table_end > size || size > profile.len() {
        return None;
    }
    let illuminant = [
        fixed(profile, 68)?,
        fixed(profile, 72)?,
        fixed(profile, 76)?,
    ];
    if !illuminant
        .iter()
        .zip(xyz((0.34567, 0.35850)))
        .all(|(a, b)| (*a - b).abs() < 1.0 / 2048.0)
    {
        return None;
    }
    let mut tags = BTreeMap::new();
    for i in 0..count {
        let index = 132 + i * 12;
        let key: [u8; 4] = profile.get(index..index + 4)?.try_into().ok()?;
        let offset = be32(profile, index + 4)? as usize;
        let length = be32(profile, index + 8)? as usize;
        let end = offset.checked_add(length)?;
        if offset < table_end || !offset.is_multiple_of(4) || length < 12 || end > size {
            return None;
        }
        tags.insert(key, profile.get(offset..end)?);
    }
    let required: &[[u8; 4]] = if gray {
        &[*b"wtpt", *b"kTRC"]
    } else {
        &[
            *b"wtpt", *b"rXYZ", *b"gXYZ", *b"bXYZ", *b"rTRC", *b"gTRC", *b"bTRC",
        ]
    };
    if required.iter().any(|key| !tags.contains_key(key)) {
        return None;
    }
    let xyz_tag = |key: &[u8; 4]| -> Option<[f32; 3]> {
        let tag = *tags.get(key)?;
        if tag.len() < 20 || tag.get(..4)? != b"XYZ " {
            return None;
        }
        Some([fixed(tag, 8)?, fixed(tag, 12)?, fixed(tag, 16)?])
    };
    let (mut matrix, keys) = if gray {
        let white = xyz_tag(b"wtpt")?;
        if (white[1] - 1.0).abs() > 1e-5 || (1.0 + white[2]) + white[0] == 0.0 {
            return None;
        }
        let matrix = if white
            .iter()
            .zip(xyz((0.31271, 0.32902)))
            .all(|(a, b)| (*a - b).abs() < 1.0 / 2048.0)
        {
            matrix(PRIMARIES[0])
        } else {
            let y = 1.0 / ((1.0 + white[2]) + white[0]);
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
        (matrix, [*b"kTRC"; 3])
    } else {
        let matrix = [xyz_tag(b"rXYZ")?, xyz_tag(b"gXYZ")?, xyz_tag(b"bXYZ")?];
        let keys = if [*b"aarg", *b"aagg", *b"aabg"]
            .iter()
            .all(|key| tags.contains_key(key))
        {
            [*b"aarg", *b"aagg", *b"aabg"]
        } else {
            [*b"rTRC", *b"gTRC", *b"bTRC"]
        };
        (matrix, keys)
    };
    if !gray {
        for (points, known) in PRIMARIES.into_iter().zip(RECOGNIZED) {
            if matrix
                .iter()
                .flatten()
                .zip(known.iter().flatten())
                .all(|(a, b)| (a - b).abs() < 1.0 / 2048.0)
            {
                matrix = self::matrix(points);
                break;
            }
        }
    }
    let mut curves = [
        curve(tags.get(&keys[0])?)?,
        curve(tags.get(&keys[1])?)?,
        curve(tags.get(&keys[2])?)?,
    ];
    if det(matrix).abs() <= 1e-5 {
        return None;
    }
    if let [Curve::Function(a), Curve::Function(b), Curve::Function(c)] = &curves
        && matches(a, b)
        && matches(a, c)
    {
        let mut f = *a;
        if [f[0], f[1], f[3], f[4]]
            .iter()
            .zip([1.0, 0.0, 0.0, 0.0])
            .all(|(a, b)| (*a - b).abs() <= 1.0 / 512.0)
        {
            f = gamma(if (f[6] - 1.0).abs() <= 1e-5 {
                1.0
            } else {
                f[6]
            });
        } else if matches(&f, &srgb()) {
            f = srgb();
        }
        curves = [Curve::Function(f), Curve::Function(f), Curve::Function(f)];
    }
    Some((matrix, curves))
}
fn apply(f: [f32; 7], x: f32) -> f32 {
    let [a, b, c, d, e, f, g] = f;
    let base = a * x + b;
    let powered = f64::from(base).powf(f64::from(g)) as f32;
    if x < d { c * x + f } else { powered + e }
}
fn lut(curve: &Curve) -> Option<[u16; 256]> {
    let mut result = [0; 256];
    for (i, out) in result.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        *out = match curve {
            Curve::Table(values) => {
                let at = x * (values.len() - 1) as f32;
                let lo = at.floor() as usize;
                let hi = (lo + 1).min(values.len() - 1);
                let frac = at - lo as f32;
                let y = (values[lo] * (1.0 - frac) + values[hi] * frac) * (1.0 / 65535.0);
                (y * 65280.0 + 0.5).floor().clamp(0.0, 65280.0) as u16
            }
            Curve::Function(f) => {
                let y = apply(*f, x);
                if !y.is_finite() {
                    return None;
                }
                let scaled = y * 65280.0;
                let negative_base = (scaled - 1.0).trunc();
                let rounded = if scaled < 0.0 {
                    (scaled - negative_base + 0.5).trunc() + negative_base
                } else {
                    (scaled + 0.5).trunc()
                };
                rounded.rem_euclid(65536.0) as u16
            }
        };
    }
    Some(result)
}
fn target_lut() -> &'static [u16; 4081] {
    static LUT: std::sync::OnceLock<[u16; 4081]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let [a, b, c, d, e, f, g] = srgb();
        let inv_a = f64::from(1.0 / a).powf(f64::from(g)) as f32;
        let inverse = [
            inv_a,
            -inv_a * e,
            1.0 / c,
            c * d + f,
            -b / a,
            -f / c,
            1.0 / g,
        ];
        std::array::from_fn(|i| {
            let x = (i as f64 / 4080.0) as f32;
            (apply(inverse, x) * 65280.0 + 0.5).floor() as u16
        })
    })
}
/// Transform 8-bit RGB or grayscale pixels. Unsupported profiles preserve both
/// the exact source pixels and profile bytes, as Qt's no-op path does.
pub fn convert_to_srgb<'a>(
    pixels: &'a [u8],
    channels: usize,
    profile: &'a [u8],
) -> (Cow<'a, [u8]>, Cow<'a, [u8]>) {
    let unchanged = || (Cow::Borrowed(pixels), Cow::Borrowed(profile));
    if profile.is_empty() || !matches!(channels, 1 | 3) || !pixels.len().is_multiple_of(channels) {
        return unchanged();
    }
    let Some((matrix, curves)) = parse(profile) else {
        return unchanged();
    };
    let target = matrix_from_srgb();
    if matrix
        .iter()
        .flatten()
        .zip(target.iter().flatten())
        .all(|(a, b)| (a - b).abs() < 1.0 / 2048.0)
        && curves
            .iter()
            .all(|curve| matches!(curve,Curve::Function(f) if matches(f,&srgb())))
    {
        return unchanged();
    }
    let transform = multiply(inverse(target), matrix);
    if det(transform).abs() <= 1e-5 {
        return unchanged();
    }
    let Some(mut tables) = curves.iter().map(lut).collect::<Option<Vec<_>>>() else {
        return unchanged();
    };
    if curves.iter().skip(1).all(|c| same(&curves[0], c)) {
        tables[1] = tables[0];
        tables[2] = tables[0];
    }
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let transform_needed = !transform
        .iter()
        .flatten()
        .zip(identity.iter().flatten())
        .all(|(a, b)| (a - b).abs() < 1.0 / 2048.0);
    let mut converted = Vec::with_capacity(pixels.len());
    for sample in pixels.chunks_exact(channels) {
        let mut linear = std::array::from_fn(|i| {
            f32::from(tables[i][usize::from(sample[if channels == 1 { 0 } else { i }])])
                * (1.0 / 65280.0)
        });
        if transform_needed {
            linear = map(transform, linear).map(|v| v.clamp(0.0, 1.0));
        }
        let indices = linear.map(|v| (v * 4080.0).round_ties_even());
        if indices
            .iter()
            .any(|n| !n.is_finite() || *n < 0.0 || *n > 4080.0)
        {
            return unchanged();
        }
        let rgb = indices.map(|i| ((u32::from(target_lut()[i as usize]) + 128) >> 8) as u8);
        if channels == 1 {
            converted.push(
                ((u16::from(rgb[0]) * 11 + u16::from(rgb[1]) * 16 + u16::from(rgb[2]) * 5) >> 5)
                    as u8,
            );
        } else {
            converted.extend_from_slice(&rgb);
        }
    }
    let profile = hex::decode(SRGB_PROFILE_HEX).unwrap_or_default();
    (Cow::Owned(converted), Cow::Owned(profile))
}
fn matrix_from_srgb() -> Matrix {
    matrix(PRIMARIES[0])
}
