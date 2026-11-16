#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::assets::{ASSET_NAMES, asset_bytes};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};
fn files(folder: &Path, root: &Path, result: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, root, result);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            result.insert(name, hex::encode(Sha256::digest(fs::read(path).unwrap())));
        }
    }
}
#[test]
fn native_export_source_and_original_assets_have_explicit_provenance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("tests/fixtures/tdexport-source-rust.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["native_source_status"],
        "rewritten-or-adapted Rust; project integrity hashes, not unchanged upstream source"
    );
    assert_eq!(manifest["reference"]["desktop_tag"], "v7.2.10");
    assert_eq!(
        manifest["reference"]["prior_handoff_commit"],
        "96a75d3d34668d4f9ff60a525c108dbbfd3d7586"
    );
    let mut actual = BTreeMap::new();
    files(&root.join("src/export"), root, &mut actual);
    let expected: BTreeMap<String, String> =
        serde_json::from_value(manifest["native_files_sha256"].clone()).unwrap();
    assert_eq!(
        actual, expected,
        "native source integrity manifest requires a reviewed update"
    );
    let assets: BTreeMap<String, String> =
        serde_json::from_value(manifest["current_emitted_assets_sha256"].clone()).unwrap();
    let baseline: BTreeMap<String, String> =
        serde_json::from_value(manifest["baseline_emitted_assets_sha256"].clone()).unwrap();
    let unchanged: BTreeMap<String, String> =
        serde_json::from_value(manifest["unchanged_emitted_assets_sha256"].clone()).unwrap();
    assert_eq!(baseline.len(), 44);
    assert_eq!(unchanged.len(), 43);
    assert_eq!(manifest["current_reference"]["desktop_tag"], "v7.4.4");
    assert_eq!(
        manifest["current_reference"]["desktop_commit"],
        "bf4f714036f21c750d535dab199c811b3f9bfdd4"
    );
    assert_eq!(
        manifest["current_reference"]["css_source_git_blob"],
        "90506ecf5b4babb8fa91184130e062dadb872500"
    );
    for (name, hash) in &unchanged {
        assert_eq!(baseline[name], *hash);
        assert_eq!(assets[name], *hash);
    }
    assert_eq!(ASSET_NAMES.len(), 44);
    assert_eq!(
        assets.keys().map(String::as_str).collect::<Vec<_>>(),
        ASSET_NAMES
    );
    for name in ASSET_NAMES {
        assert_eq!(
            hex::encode(Sha256::digest(asset_bytes(name).unwrap())),
            assets[*name],
            "original emitted asset {name}"
        );
    }
}
#[test]
fn source_package_includes_native_assets_and_complete_legal_notices() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let include = cargo.split_once("include = ").unwrap().1;
    let include = include
        .split_once('[')
        .unwrap()
        .1
        .split_once(']')
        .unwrap()
        .0;
    let include: Vec<String> =
        serde_json::from_str(&format!("[{}]", include.trim().trim_end_matches(','))).unwrap();
    for name in ["src/**", "NOTICE", "LICENSE"] {
        assert!(
            include.iter().any(|v| v == name),
            "source package omits {name}"
        );
    }
    let module = fs::read_to_string(root.join("src/export/assets.rs")).unwrap();
    assert!(module.contains("include_bytes!"));
    for name in ASSET_NAMES {
        assert!(asset_bytes(name).is_some());
    }
    let notice = fs::read_to_string(root.join("NOTICE")).unwrap();
    for required in [
        "TELEGRAM DESKTOP EXPORT",
        "Copyright (C) 2004, 2005 Daniel M. Duley",
        "src/export/qt_icc.rs",
        "src/export/qt_scale.rs",
        "src/export/qt_jpeg.rs",
        "image-rs/jpeg-decoder",
        "mozjpeg-rs",
    ] {
        assert!(notice.contains(required), "missing provenance {required}");
    }
    for license in [
        "src/export/jpeg_decoder/LICENSE-MIT",
        "src/export/jpeg_decoder/LICENSE-APACHE",
    ] {
        assert!(fs::read(root.join(license)).unwrap().len() > 500);
    }
}
