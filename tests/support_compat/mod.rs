#![allow(clippy::unwrap_used)]
pub use super::source_files::walk as source_files;
use std::{path::Path, process::Command, sync::OnceLock};
pub fn metadata() -> &'static serde_json::Value {
    static DATA: OnceLock<serde_json::Value> = OnceLock::new();
    DATA.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["metadata", "--offline", "--locked", "--format-version", "1"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "locked Cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    })
}
pub fn package(name: &str) -> &'static serde_json::Value {
    let packages = metadata()["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == name)
        .collect::<Vec<_>>();
    assert_eq!(
        packages.len(),
        1,
        "SDK package must have one canonical source graph: {name}"
    );
    packages[0]
}
pub fn dependency_source(name: &str) -> String {
    let path = Path::new(package(name)["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("src/lib.rs");
    std::fs::read_to_string(path).unwrap()
}
