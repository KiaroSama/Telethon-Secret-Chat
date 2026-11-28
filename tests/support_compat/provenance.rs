#![allow(clippy::unwrap_used)]
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};
include!(concat!(env!("OUT_DIR"), "/bundled_sdk.rs"));
pub fn verify() {
    let embedded: BTreeMap<_, _> = SDK_FILES.iter().copied().collect();
    assert_eq!(embedded.len(), SDK_FILES.len());
    let native: serde_json::Value = serde_json::from_slice(embedded["local-patches.json"]).unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(embedded["upstream-hashes.json"]).unwrap();
    assert_eq!(native["version"], 1);
    assert_eq!(native["upstream_revision"], original["revision"]);
    assert_eq!(
        native["upstream_revision"],
        "11e13a989a754f9cf01a798480a2e20ce9e5b1fd"
    );
    assert_eq!(
        native["baseline_manifest_sha256"],
        hex::encode(Sha256::digest(embedded["upstream-hashes.json"]))
    );
    let declared: BTreeMap<String, String> =
        serde_json::from_value(native["native_files_sha256"].clone()).unwrap();
    assert_eq!(
        declared.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        embedded
            .keys()
            .copied()
            .filter(|p| *p != "local-patches.json")
            .collect::<BTreeSet<_>>()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/grammers");
    for (path, expected) in &declared {
        assert!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        );
        assert_eq!(
            hex::encode(Sha256::digest(embedded[path.as_str()])),
            *expected,
            "embedded patched SDK bytes: {path}"
        );
        assert_eq!(
            hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
            *expected,
            "installed patched SDK bytes: {path}"
        );
    }
    let rust_files = super::source_files::walk(&root)
        .into_iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        rust_files,
        declared
            .keys()
            .filter(|path| path.ends_with(".rs"))
            .cloned()
            .collect::<BTreeSet<_>>()
    );
    let baseline: BTreeMap<String, String> =
        serde_json::from_value(original["sha256"].clone()).unwrap();
    assert_eq!(
        baseline["grammers-tl-types/tl/api.tl"],
        "0dc1c3ee1976ecf352d8af005eca949d89afa10aff4b067a8f8304e2f37f7ffb"
    );
    let patched = baseline
        .iter()
        .filter(|(path, _)| path.ends_with(".rs"))
        .filter_map(|(path, hash)| (declared[path] != *hash).then_some(path.clone()))
        .collect::<BTreeSet<_>>();
    let supplements = declared
        .keys()
        .filter(|path| path.ends_with(".rs") && !baseline.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        patched,
        serde_json::from_value::<BTreeSet<String>>(native["patched_upstream_sources"].clone())
            .unwrap()
    );
    assert_eq!(
        supplements,
        serde_json::from_value::<BTreeSet<String>>(native["supplement_sources"].clone()).unwrap()
    );
}
