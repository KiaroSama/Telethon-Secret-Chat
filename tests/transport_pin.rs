use std::path::PathBuf;

use sha2::{Digest, Sha256};

const EXPECTED_UPSTREAM_REV: &str = "11e13a989a754f9cf01a798480a2e20ce9e5b1fd";
const SECRET_CHAT_INNER_LAYERS: [i32; 5] = [46, 73, 101, 144, 200];

#[test]
fn outer_telegram_api_layer_stays_separate_from_secret_chat_layers() {
    assert_eq!(grammers_tl_types::LAYER, 234);
    assert!(!SECRET_CHAT_INNER_LAYERS.contains(&grammers_tl_types::LAYER));
}

#[test]
fn vendor_grammers_manifests_match_pinned_revision_and_disk_hashes() {
    let vendor_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("vendor")
        .join("grammers");
    let upstream_bytes = std::fs::read(vendor_dir.join("upstream-hashes.json"))
        .expect("read vendor/grammers/upstream-hashes.json");
    let local_bytes = std::fs::read(vendor_dir.join("local-patches.json"))
        .expect("read vendor/grammers/local-patches.json");

    let upstream: serde_json::Value =
        serde_json::from_slice(&upstream_bytes).expect("parse upstream-hashes.json");
    let local: serde_json::Value =
        serde_json::from_slice(&local_bytes).expect("parse local-patches.json");

    assert_eq!(upstream["revision"], EXPECTED_UPSTREAM_REV);
    assert_eq!(local["upstream_revision"], EXPECTED_UPSTREAM_REV);

    let actual_baseline_sha = hex::encode(Sha256::digest(&upstream_bytes));
    assert_eq!(local["baseline_manifest_sha256"], actual_baseline_sha);

    let native_hashes = local["native_files_sha256"]
        .as_object()
        .expect("native_files_sha256 object");
    assert!(!native_hashes.is_empty());
    for (rel_path, expected_sha) in native_hashes {
        let data = std::fs::read(vendor_dir.join(rel_path))
            .unwrap_or_else(|err| panic!("missing vendored file {rel_path}: {err}"));
        let actual_sha = hex::encode(Sha256::digest(&data));
        assert_eq!(
            expected_sha.as_str(),
            Some(actual_sha.as_str()),
            "hash mismatch for vendor/grammers/{rel_path}"
        );
    }
}
