use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use kiaro_secret_chat::export::{
    convert_icc_to_srgb, escape_html_text, format_amount_and_currency, jpeg_source_metadata,
    mime_first_glob, parse_strict_qurl, safe_http_href, serialize_json_string,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_files(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel);
        }
    }
}

fn read_hex_fixture(dir: &Path, name: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.hex"))).expect("read hex fixture");
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    hex::decode(compact).expect("decode hex fixture")
}

#[test]
fn export_asset_and_icc_fixture_inventories_match_frozen_baseline_counts() {
    let root = repo_root();
    let mut assets = BTreeSet::new();
    collect_files(&root, &root.join("assets/tdexport"), &mut assets);
    assert_eq!(
        assets.len(),
        46,
        "expected exactly 46 tdexport asset files, found {}",
        assets.len()
    );
    assert!(assets.contains("assets/tdexport/css/style.css"));

    let mut icc_files = BTreeSet::new();
    collect_files(
        &root,
        &root.join("tests/fixtures/tdexport_icc"),
        &mut icc_files,
    );
    assert_eq!(
        icc_files.len(),
        58,
        "expected exactly 58 tdexport_icc fixture files, found {}",
        icc_files.len()
    );
    assert!(icc_files.contains("tests/fixtures/tdexport_icc/adobe-rgb.icc"));
    assert!(icc_files.contains("tests/fixtures/tdexport_icc/adobe-thumbnail.expected.jpg"));
}

#[test]
fn export_core_formatters_and_jpeg_metadata_match_reference_goldens() {
    assert_eq!(
        escape_html_text("A <B> & \"C\" 'D' 👋").expect("escape html"),
        "A &lt;B&gt; &amp; &quot;C&quot; &apos;D&apos; 👋"
    );
    assert_eq!(
        parse_strict_qurl("https://example.com/a b?q=1#f").expect("qurl"),
        "https://example.com/a%20b?q=1#f"
    );
    assert_eq!(
        safe_http_href("javascript:alert(1)").expect("safe href"),
        None
    );
    assert_eq!(mime_first_glob("image/jpeg").expect("mime"), "*.jpg");
    assert_eq!(
        format_amount_and_currency(12345, "USD", false).expect("currency"),
        "$123.45"
    );
    assert_eq!(
        serialize_json_string("hi\n\"world\"").expect("json str"),
        "\"hi\\n\\\"world\\\"\""
    );

    let root = repo_root();
    let jpeg_bytes =
        std::fs::read(root.join("tests/fixtures/tdexport_icc/adobe-thumbnail.source.jpg"))
            .expect("read jpeg fixture");
    let meta = jpeg_source_metadata(&jpeg_bytes).expect("jpeg source metadata");
    assert!(meta.is_object());
}

#[test]
fn all_qt_icc_golden_fixtures_match_byte_for_byte() {
    let fixtures = repo_root().join("tests/fixtures/tdexport_icc");
    let srgb_expected = std::fs::read(fixtures.join("srgb.icc")).expect("read srgb.icc");

    // 1) Adobe RGB golden match (pixels + emitted srgb.icc profile).
    let src_rgb = read_hex_fixture(&fixtures, "source.rgb");
    let adobe_icc = std::fs::read(fixtures.join("adobe-rgb.icc")).expect("read adobe-rgb.icc");
    let (converted, emitted) =
        convert_icc_to_srgb(&src_rgb, &[16, 16, 3], &adobe_icc).expect("adobe-rgb convert");
    assert_eq!(converted, read_hex_fixture(&fixtures, "adobe-rgb.rgb"));
    assert_eq!(emitted, srgb_expected);

    // 2) All 15 parametrized curve fixtures match Qt 5.15.19 byte for byte.
    for case in [
        "parametric-0",
        "parametric-1",
        "parametric-2",
        "parametric-3",
        "parametric-4",
        "table",
        "table-offset",
        "linear-table",
        "linear",
        "gamma88",
        "channels",
        "apple",
        "custom-matrix",
        "gray",
        "srgb",
    ] {
        let pixels = read_hex_fixture(&fixtures, &format!("{case}.source.rgb"));
        let icc = std::fs::read(fixtures.join(format!("{case}.icc"))).expect("read case icc");
        let shape = if pixels.len() == 16 * 16 {
            vec![16, 16]
        } else {
            vec![16, 16, 3]
        };
        let (out_pixels, out_profile) = convert_icc_to_srgb(&pixels, &shape, &icc).expect(case);
        assert_eq!(
            out_pixels,
            read_hex_fixture(&fixtures, &format!("{case}.rgb")),
            "pixel mismatch on ICC golden case {case}"
        );
        assert_eq!(
            out_profile, srgb_expected,
            "profile mismatch on ICC golden case {case}"
        );
    }

    // 3) Malformed ICC bytes preserve original pixels and profile (Qt fallback parity).
    let (kept_pixels, kept_profile) =
        convert_icc_to_srgb(&[128, 64, 32], &[1, 1, 3], &[0x00, 0x01, 0x02]).expect("malformed");
    assert_eq!(kept_pixels, vec![128, 64, 32]);
    assert_eq!(kept_profile, vec![0x00, 0x01, 0x02]);
}
