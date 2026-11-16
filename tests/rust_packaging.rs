#![allow(clippy::unwrap_used)]
#[path = "support_package/mod.rs"]
mod package;
#[path = "support_sources/mod.rs"]
mod source_files;
#[test]
fn gpl_license_is_shipped_in_actual_git_source() {
    let p = package::installed();
    assert!(
        p.license.contains("GNU GENERAL PUBLIC LICENSE")
            && p.license.contains("Version 3, 29 June 2007")
    );
    assert!(p.source.starts_with("git+file://"));
    assert_eq!(p.revision.len(), 40);
    assert!(p.manifest_has_vendor);
}
#[test]
fn inherited_notices_travel_with_git_dependency() {
    let p = package::installed();
    for marker in [
        "MIT License",
        "Copyright (c) 2020 painor",
        "Boost Software License",
        "core.telegram.org/api/end-to-end",
    ] {
        assert!(
            p.notice.contains(marker),
            "missing inherited notice {marker}"
        );
    }
    assert!(p.notice.contains("grammers"));
    assert!(p.lock_unchanged);
}
#[test]
fn installed_consumer_exports_resolve_and_execute() {
    let p = package::installed();
    assert!(p.installed && p.canonical_sdk && p.no_source_stubs && p.standalone_tools);
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tdexport_assets_sha256_current.json")).unwrap();
    for (name, sha) in golden.as_object().unwrap() {
        assert_eq!(
            p.assets.get(name).map(String::as_str),
            sha.as_str(),
            "installed export asset {name}"
        );
    }
    assert!(p.assets.contains_key("messages.html") && p.assets.contains_key("result.json"));
}
#[test]
fn compiled_version_comes_from_distribution_metadata() {
    let p = package::installed();
    assert_eq!(p.version, p.declared_version);
    assert_eq!(p.version, kiaro_secret_chat::VERSION);
    assert_eq!(p.version, env!("CARGO_PKG_VERSION"));
}
