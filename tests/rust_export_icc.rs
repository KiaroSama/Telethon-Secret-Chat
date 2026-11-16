#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::qt_icc::convert_to_srgb;
use std::{borrow::Cow, collections::BTreeMap, path::Path};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tdexport_icc")
            .join(name),
    )
    .unwrap()
}
fn pixels(name: &str) -> Vec<u8> {
    hex::decode(
        String::from_utf8(data(name))
            .unwrap()
            .split_whitespace()
            .collect::<String>(),
    )
    .unwrap()
}
fn supported(name: &str) {
    let source = if name == "adobe-rgb" {
        "source.rgb.hex".into()
    } else {
        format!("{name}.source.rgb.hex")
    };
    let input = pixels(&source);
    let profile = data(&format!("{name}.icc"));
    let (actual, emitted) = convert_to_srgb(&input, 3, &profile);
    assert_eq!(&*actual, pixels(&format!("{name}.rgb.hex")));
    assert_eq!(&*emitted, data("srgb.icc"));
    if name == "srgb" {
        assert!(matches!(actual, Cow::Borrowed(_)));
        assert!(matches!(emitted, Cow::Borrowed(_)));
    }
}
fn tags(profile: &[u8]) -> BTreeMap<[u8; 4], (usize, usize)> {
    let count = u32::from_be_bytes(profile[128..132].try_into().unwrap()) as usize;
    (0..count)
        .map(|i| {
            let pos = 132 + i * 12;
            (
                profile[pos..pos + 4].try_into().unwrap(),
                (
                    pos,
                    u32::from_be_bytes(profile[pos + 4..pos + 8].try_into().unwrap()) as usize,
                ),
            )
        })
        .collect()
}
fn put32(profile: &mut [u8], at: usize, value: u32) {
    profile[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
fn reject(mutation: &str) {
    let mut profile = data("parametric-1.icc");
    let locations = tags(&profile);
    let (index, offset) = locations[b"rTRC"];
    match mutation {
        "empty" => profile.clear(),
        "truncated" => {
            profile.pop();
        }
        "signature" => profile[36..40].copy_from_slice(b"nope"),
        "class" => profile[12..16].copy_from_slice(b"link"),
        "cmyk" => profile[16..20].copy_from_slice(b"CMYK"),
        "lab" => profile[20..24].copy_from_slice(b"Lab "),
        "illuminant" => profile[68..80].fill(0),
        "count" => put32(&mut profile, 128, u32::MAX),
        "offset" => put32(&mut profile, 136, 128),
        "alignment" => put32(&mut profile, 136, locations[b"rXYZ"].1 as u32 + 1),
        "tag-size" => put32(&mut profile, 140, 8),
        "missing-matrix" => profile[132..136].copy_from_slice(b"A2B0"),
        "xyz-type" => {
            let p = locations[b"rXYZ"].1;
            profile[p..p + 4].copy_from_slice(b"Lab ");
        }
        "singular" => {
            for key in [b"rXYZ", b"gXYZ", b"bXYZ"] {
                let p = locations[key].1;
                profile[p + 8..p + 20].fill(0);
            }
        }
        "curve-type" => profile[offset..offset + 4].copy_from_slice(b"mft2"),
        "parametric-type" => profile[offset + 8..offset + 10].copy_from_slice(&5u16.to_be_bytes()),
        "short-parameters" => put32(&mut profile, index + 8, 16),
        "zero-a" => put32(&mut profile, offset + 16, 0),
        "table-count" | "table-order" => {
            profile = data("table.icc");
            let p = u32::from_be_bytes(profile[index + 4..index + 8].try_into().unwrap()) as usize;
            if mutation == "table-count" {
                put32(&mut profile, p + 8, 65537);
            } else {
                profile[p + 12..p + 16].copy_from_slice(&[255, 255, 0, 0]);
            }
        }
        _ => panic!("Unknown mutation"),
    }
    let input = [0; 12];
    let (actual, emitted) = convert_to_srgb(&input, 3, &profile);
    assert!(matches!(actual, Cow::Borrowed(_)));
    assert!(matches!(emitted, Cow::Borrowed(_)));
    assert_eq!(&*actual, input);
    assert_eq!(&*emitted, profile);
}
fn gray(name: &str) {
    let input = pixels(&format!("{name}.source.gray.hex"));
    let profile = data(&format!("{name}.icc"));
    let (actual, emitted) = convert_to_srgb(&input, 1, &profile);
    assert_eq!(actual.len(), input.len());
    assert_eq!(&*actual, pixels(&format!("{name}.gray.hex")));
    assert_eq!(&*emitted, data("srgb.icc"));
}
#[test]
fn out_of_range_parametric_curve() {
    let mut profile = data("parametric-4.icc");
    let srgb = data("srgb.icc");
    let locations = tags(&profile);
    let target = tags(&srgb);
    for key in [b"rXYZ", b"gXYZ", b"bXYZ"] {
        let p = locations[key].1;
        let q = target[key].1;
        profile[p..p + 20].copy_from_slice(&srgb[q..q + 20]);
    }
    for key in [b"rTRC", b"gTRC", b"bTRC"] {
        let p = locations[key].1;
        profile[p + 8..p + 10].copy_from_slice(&4u16.to_be_bytes());
        for (i, v) in [65536, 65536, 0, 65536, 0, 256, 0].into_iter().enumerate() {
            put32(&mut profile, p + 12 + i * 4, v);
        }
    }
    let pixels = [255; 3];
    let (actual, emitted) = convert_to_srgb(&pixels, 3, &profile);
    assert!(matches!(actual, Cow::Borrowed(_)));
    assert!(matches!(emitted, Cow::Borrowed(_)));
    assert_eq!(&*actual, pixels);
    assert_eq!(&*emitted, profile);
}

#[test]
fn adobe_rgb() {
    supported("adobe-rgb");
}

#[test]
fn supported_parametric_0() {
    supported("parametric-0");
}

#[test]
fn supported_parametric_1() {
    supported("parametric-1");
}

#[test]
fn supported_parametric_2() {
    supported("parametric-2");
}

#[test]
fn supported_parametric_3() {
    supported("parametric-3");
}

#[test]
fn supported_parametric_4() {
    supported("parametric-4");
}

#[test]
fn supported_table() {
    supported("table");
}

#[test]
fn supported_table_offset() {
    supported("table-offset");
}

#[test]
fn supported_linear_table() {
    supported("linear-table");
}

#[test]
fn supported_linear() {
    supported("linear");
}

#[test]
fn supported_gamma88() {
    supported("gamma88");
}

#[test]
fn supported_channels() {
    supported("channels");
}

#[test]
fn supported_apple() {
    supported("apple");
}

#[test]
fn supported_custom_matrix() {
    supported("custom-matrix");
}

#[test]
fn supported_gray() {
    supported("gray");
}

#[test]
fn supported_srgb() {
    supported("srgb");
}

#[test]
fn reject_empty() {
    reject("empty");
}

#[test]
fn reject_truncated() {
    reject("truncated");
}

#[test]
fn reject_signature() {
    reject("signature");
}

#[test]
fn reject_class() {
    reject("class");
}

#[test]
fn reject_cmyk() {
    reject("cmyk");
}

#[test]
fn reject_lab() {
    reject("lab");
}

#[test]
fn reject_illuminant() {
    reject("illuminant");
}

#[test]
fn reject_count() {
    reject("count");
}

#[test]
fn reject_offset() {
    reject("offset");
}

#[test]
fn reject_alignment() {
    reject("alignment");
}

#[test]
fn reject_tag_size() {
    reject("tag-size");
}

#[test]
fn reject_missing_matrix() {
    reject("missing-matrix");
}

#[test]
fn reject_xyz_type() {
    reject("xyz-type");
}

#[test]
fn reject_singular() {
    reject("singular");
}

#[test]
fn reject_curve_type() {
    reject("curve-type");
}

#[test]
fn reject_parametric_type() {
    reject("parametric-type");
}

#[test]
fn reject_short_parameters() {
    reject("short-parameters");
}

#[test]
fn reject_zero_a() {
    reject("zero-a");
}

#[test]
fn reject_table_count() {
    reject("table-count");
}

#[test]
fn reject_table_order() {
    reject("table-order");
}

#[test]
fn gray_gray_jpeg() {
    gray("gray-jpeg");
}

#[test]
fn gray_gray_adobe() {
    gray("gray-adobe");
}
