#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    qt_jpeg,
    qt_scale::{down_taps, smooth_scale, up_taps},
    thumbnail::{ThumbnailEncoder, ThumbnailOptions, write_image_thumb},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tdexport_thumbs")
            .join(format!("{name}.source.jpg")),
    )
    .unwrap()
}
fn segments(bytes: &[u8]) -> Vec<(u8, &[u8])> {
    assert_eq!(&bytes[..2], b"\xff\xd8");
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= bytes.len() {
        let marker = bytes[i + 1];
        let size = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        assert!(size >= 2 && i + 2 + size <= bytes.len());
        out.push((marker, &bytes[i + 4..i + 2 + size]));
        if marker == 218 {
            return out;
        }
        i += 2 + size;
    }
    panic!("JPEG has no complete scan");
}
fn thumb(source: &[u8], size: (u32, u32), options: &ThumbnailOptions<'_>) -> Vec<u8> {
    let folder = tempfile::tempdir().unwrap();
    fs::create_dir(folder.path().join("photos")).unwrap();
    fs::write(folder.path().join("photos/p.jpg"), source).unwrap();
    let (path, actual) = write_image_thumb(folder.path(), "photos/p.jpg", |_| size, options);
    assert_eq!(actual, size);
    fs::read(folder.path().join(path)).unwrap()
}
fn desktop(source: &[u8], size: (u32, u32)) -> Vec<u8> {
    thumb(source, size, &ThumbnailOptions::default())
}
#[test]
fn down_taps_follow_qimage_calc_apoints() {
    assert_eq!(
        down_taps(4, 2).unwrap(),
        vec![(0, vec![8192, 8192]), (2, vec![8192, 8192])]
    );
    assert_eq!(
        down_taps(3, 2).unwrap(),
        vec![(0, vec![10923, 5461]), (1, vec![5461, 10923])]
    );
    for source in 2..90 {
        for target in 1..source {
            for (start, weights) in down_taps(source, target).unwrap() {
                assert_eq!(weights.iter().sum::<i64>(), 1 << 14);
                assert!(start + weights.len() <= source);
            }
        }
    }
}
#[test]
fn up_taps_follow_qimage_calc_apoints() {
    assert_eq!(
        up_taps(2, 4).unwrap(),
        vec![
            (0, vec![256, 0]),
            (0, vec![192, 64]),
            (0, vec![64, 192]),
            (1, vec![256, 0])
        ]
    );
}
#[test]
fn smooth_scale_floors_box_average() {
    let pixels = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3];
    assert_eq!(smooth_scale(&pixels, 2, 2, 3, 1, 1).unwrap(), vec![1, 1, 1]);
}
#[test]
fn smooth_scale_down_x_only() {
    let pixels = [0, 0, 0, 100, 100, 100, 200, 200, 200];
    assert_eq!(
        smooth_scale(&pixels, 3, 1, 3, 2, 1).unwrap(),
        vec![33, 33, 33, 166, 166, 166]
    );
}
#[test]
fn smooth_scale_up_interpolates_and_truncates() {
    // A small wide source must be refused before a multi-gigabyte intermediate.
    // Direct final-byte output makes the former one-byte grayscale case fit.
    let wide_source = vec![0; 10000 * 4];
    assert!(smooth_scale(&wide_source, 10000, 1, 4, 10000, 10000).is_err());

    assert_eq!(
        smooth_scale(&[0, 0, 0, 200, 200, 200], 2, 1, 3, 4, 1).unwrap(),
        vec![0, 0, 0, 50, 50, 50, 150, 150, 150, 200, 200, 200]
    );
    let pixels = [0, 0, 0, 255, 255, 255, 100, 100, 100, 0, 0, 0];
    assert_eq!(
        smooth_scale(&pixels, 2, 2, 3, 4, 4).unwrap()[(4 + 1) * 3],
        66
    );
}
#[test]
fn density_fields_match_qjpeghandler() {
    assert_eq!(qt_jpeg::density_fields(3780, 3780), (1, 96, 96));
    assert_eq!(qt_jpeg::density_fields(2834, 2834), (1, 72, 72));
    assert_eq!(qt_jpeg::density_fields(2800, 2800), (2, 28, 28));
}
#[test]
fn source_metadata_reads_density_and_icc() {
    let profile = vec![7; 65533 - 14 + 1];
    let encoded = qt_jpeg::splice(&fixture("small"), (3780, 3780), &profile, &[]).unwrap();
    assert_eq!(qt_jpeg::source_metadata(&encoded).1, profile);

    let mut source =
        b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x01\x00\x48\x00\x48\x00\x00\xff\xe2".to_vec();
    source.extend(216u16.to_be_bytes());
    source.extend_from_slice(b"ICC_PROFILE\0\x01\x01");
    source.extend([0; 200]);
    source.extend_from_slice(b"\xff\xda\x00\x02");
    assert_eq!(
        qt_jpeg::source_metadata(&source),
        ((2834, 2834), vec![0; 200])
    );
    source[13] = 0;
    assert_eq!(qt_jpeg::source_metadata(&source).0, (3780, 3780));
    assert_eq!(
        qt_jpeg::source_metadata(b"\xff\xd8\xff\xda\x00\x02"),
        ((3780, 3780), vec![])
    );
}
#[test]
fn thumb_is_mozjpeg_progressive_444_with_desktop_tables() {
    let bytes = desktop(&fixture("rgb"), (40, 30));
    let parts = segments(&bytes);
    assert_eq!(
        parts[0],
        (224, &b"JFIF\0\x01\x01\x01\x00\x60\x00\x60\x00\x00"[..])
    );
    assert!(!parts.iter().any(|(m, _)| matches!(*m, 226 | 254 | 192)));
    let sof = parts.iter().find(|(m, _)| *m == 194).unwrap().1;
    assert_eq!(&sof[1..5], &[0, 30, 0, 40]);
    assert_eq!([sof[7], sof[10], sof[13]], [17; 3]);
    let tables = parts
        .iter()
        .filter(|(m, _)| *m == 219)
        .map(|(_, p)| *p)
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].len(), 130);
    let master = [
        16, 16, 16, 18, 25, 37, 56, 85, 16, 17, 20, 27, 34, 40, 53, 75, 16, 20, 24, 31, 43, 62, 91,
        135, 18, 27, 31, 40, 53, 74, 106, 156, 25, 34, 43, 53, 69, 94, 131, 189, 37, 40, 62, 74,
        94, 124, 169, 238, 56, 53, 91, 106, 131, 169, 226, 311, 85, 75, 135, 156, 189, 238, 311,
        418,
    ];
    let zigzag = [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27,
        20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
        58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ];
    let expected = zigzag.map(|z| ((master[z] * 50 + 50) / 100).clamp(1, 255) as u8);
    assert_eq!(tables[0][0], 0);
    assert_eq!(&tables[0][1..65], &expected);
    assert_eq!(tables[0][65], 1);
    assert_eq!(&tables[0][66..130], &expected);
}
#[test]
fn thumb_carries_source_density_and_icc() {
    for (fixture_name, color, size) in [
        ("cmyk", *b"CMYK", (8, 8)),
        ("gray", *b"GRAY", (8, 8)),
        ("rgb", *b"RGB ", (8, 8)),
    ] {
        let mut unsupported = vec![0; 200];
        unsupported[16..20].copy_from_slice(&color);
        let original = fixture(fixture_name);
        let mut source = original[..2].to_vec();
        source.extend([255, 226]);
        source.extend(((unsupported.len() + 16) as u16).to_be_bytes());
        source.extend_from_slice(b"ICC_PROFILE\0\x01\x01");
        source.extend_from_slice(&unsupported);
        source.extend_from_slice(&original[2..]);
        let output = desktop(&source, size);
        assert_eq!(
            qt_jpeg::source_metadata(&output).1,
            if color == *b"RGB " {
                unsupported
            } else {
                vec![]
            }
        );
    }
    let source = fixture("density");
    let bytes = desktop(&source, (25, 20));
    let parts = segments(&bytes);
    assert_eq!(&parts[0].1[7..12], b"\x01\x00\x48\x00\x48");
    let original = segments(&source);
    let profile = original.iter().find(|(m, _)| *m == 226).unwrap().1;
    assert_eq!(parts[1], (226, profile));
}
#[test]
fn unscaled_grayscale_stays_one_component() {
    let source = fixture("gray");
    let bytes = desktop(&source, (16, 16));
    assert_eq!(
        segments(&bytes).iter().find(|(m, _)| *m == 194).unwrap().1[5],
        1
    );
    let bytes = desktop(&source, (8, 8));
    assert_eq!(
        segments(&bytes).iter().find(|(m, _)| *m == 194).unwrap().1[5],
        3
    );
}
#[test]
fn without_mozjpeg_baseline_still_writes_a_thumb() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("oversized.jpg"), fixture("small")).unwrap();
    let source = fs::OpenOptions::new()
        .write(true)
        .open(folder.path().join("oversized.jpg"))
        .unwrap();
    source.set_len(64 * 1024 * 1024 + 1).unwrap();
    assert_eq!(
        image::ImageReader::open(folder.path().join("oversized.jpg"))
            .unwrap()
            .into_dimensions()
            .unwrap(),
        (16, 16)
    );
    assert_eq!(
        write_image_thumb(
            folder.path(),
            "oversized.jpg",
            |_| panic!("oversized input reached geometry callback"),
            &ThumbnailOptions::default()
        ),
        (String::new(), (0, 0))
    );
    let options = ThumbnailOptions {
        jpeg_encoder: ThumbnailEncoder::Baseline,
        ..Default::default()
    };
    let bytes = thumb(&fixture("rgb"), (40, 30), &options);
    assert!(segments(&bytes).iter().any(|(m, _)| *m == 192));
}
fn cmyk(size: (u32, u32)) {
    let bytes = desktop(&fixture("cmyk"), size);
    assert!(segments(&bytes).iter().any(|(m, _)| *m == 194));
    let rgb = image::load_from_memory(&bytes).unwrap().into_rgb8();
    let actual = rgb.get_pixel(0, 0).0;
    let expected = [70, 130, 190].map(|c| ((255 - 90) * (255 - c) / 255) as u8);
    for (a, b) in actual.into_iter().zip(expected) {
        assert!(a.abs_diff(b) <= 2, "{a} != {b}");
    }
}
#[test]
fn cmyk_thumbnail_uses_desktop_rgb_conversion_16() {
    cmyk((16, 16));
}
#[test]
fn cmyk_thumbnail_uses_desktop_rgb_conversion_8() {
    cmyk((8, 8));
}
#[test]
fn jpeg_comments_use_qt_key_order_and_utf8() {
    let source = fixture("small");
    let mut comments = Vec::new();
    for text in [
        "Zeta: final",
        "a description",
        "Alpha: first",
        "Zeta: replaced",
    ] {
        comments.extend([255, 254]);
        comments.extend(((text.len() + 2) as u16).to_be_bytes());
        comments.extend_from_slice(text.as_bytes());
    }
    let mut marked = source[..2].to_vec();
    marked.extend(comments);
    marked.extend_from_slice(&source[2..]);
    let bytes = desktop(&marked, (8, 8));
    let comments = segments(&bytes)
        .into_iter()
        .filter(|(m, _)| *m == 254)
        .map(|(_, p)| p)
        .collect::<Vec<_>>();
    assert_eq!(
        comments,
        vec![
            b"Alpha: first" as &[u8],
            b"Description: a description",
            b"Zeta: replaced"
        ]
    );
}
#[test]
fn non_srgb_thumbnail_matches_exact_qt_oracle() {
    // Current Desktop explicitly rejects SVG/SVGZ before thumbnail decoding.
    // The native raster reader refuses both without invoking a vector renderer.
    let folder = tempfile::tempdir().unwrap();
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\"/></svg>";
    for (name, bytes) in [("untrusted.svg", svg.as_slice()), ("untrusted.svgz", b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\x03\xb3\x29\x2e\x4b\x57\xa8\xc8\xcd\xc9\x2b\xb6\x55\xca\x28\x29\x29\xb0\xd2\xd7\x2f\x2f\x2f\xd7\x2b\x37\xd6\xcb\x2f\x4a\xd7\x37\x32\x30\x30\xd0\x07\xaa\x50\x52\x28\xcf\x4c\x29\xc9\xb0\x55\xb2\x50\x52\xc8\x48\xcd\x4c\xcf\x28\x01\x31\xed\x6c\x8a\x52\x93\x4b\xb0\x4a\xe9\xdb\xd9\x80\xf4\xd9\x01\x00\x40\x2f\xca\x35\x5f\x00\x00\x00".as_slice())] {
        fs::write(folder.path().join(name), bytes).unwrap();
        assert_eq!(write_image_thumb(folder.path(), name, |_| (8,8), &ThumbnailOptions::default()), (String::new(), (0,0)));
        assert!(!folder.path().join(name.replace('.', "_thumb.")).exists());
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tdexport_icc");
    let source = fs::read(root.join("adobe-thumbnail.source.jpg")).unwrap();
    let expected = fs::read(root.join("adobe-thumbnail.expected.jpg")).unwrap();
    assert_eq!(desktop(&source, (8, 8)), expected);
}
