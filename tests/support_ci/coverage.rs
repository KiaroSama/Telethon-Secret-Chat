#![allow(clippy::unwrap_used)]
use crate::gate;
use serde_json::{Value, json};
use std::{path::Path, process::Command};
fn fixture(covered: u64) -> Value {
    let lines = json!({"count":10000,"covered":covered,"percent":covered as f64 / 100.0});
    let metrics = json!({"lines":lines,"functions":{"count":10,"covered":9,"percent":90.0},"regions":{"count":100,"covered":80,"notcovered":20,"percent":80.0}});
    json!({"type":"llvm.coverage.json.export","version":"3.1.0","data":[{"files":[{"filename":Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),"summary":metrics}],"totals":metrics}]})
}
fn check(value: &Value) -> Result<String, String> {
    gate::check_coverage(
        &serde_json::to_vec(value).unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}
pub fn valid() {
    for covered in [9000, 9103, 10000] {
        assert!(
            check(&fixture(covered))
                .unwrap()
                .contains("90% gate passed")
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("summary.json");
    std::fs::write(&path, serde_json::to_vec(&fixture(9000)).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_matrix-evidence"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--coverage")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
pub fn invalid() {
    assert!(check(&fixture(8999)).is_err());
    let mut cases = Vec::new();
    let mut value = fixture(9000);
    value["data"][0]["files"] = json!([]);
    cases.push(value);
    let mut value = fixture(9000);
    value["data"][0]["totals"]["lines"]["covered"] = json!(10001);
    cases.push(value);
    let mut value = fixture(9000);
    value["data"][0]["totals"]["lines"]["percent"] = json!(91.0);
    cases.push(value);
    let mut value = fixture(9000);
    value["data"][0]["totals"]["lines"]["count"] = json!(0);
    cases.push(value);
    let mut value = fixture(9000);
    value["data"][0]["files"][0]["summary"]["lines"] =
        json!({"count":10000,"covered":8999,"percent":89.99});
    cases.push(value);
    let mut value = fixture(9000);
    let duplicate = value["data"][0]["files"][0].clone();
    value["data"][0]["files"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    cases.push(value);
    let mut value = fixture(9000);
    value["data"][0]["totals"]["regions"]["notcovered"] = json!(1);
    cases.push(value);
    let mut value = fixture(9000);
    value["type"] = json!("wrong");
    cases.push(value);
    let mut value = fixture(9000);
    value["version"] = json!("3.invalid");
    cases.push(value);
    for name in [
        "../PUBLIC-BURNED-PATH-DO-NOT-PRINT.rs",
        "src/bin/matrix-evidence.rs",
        "tests/rust_ci_audit.rs",
        "src/storage/contract_tests.rs",
        "src/export/jpeg_decoder/parser.rs",
    ] {
        let mut value = fixture(9000);
        value["data"][0]["files"][0]["filename"] =
            json!(Path::new(env!("CARGO_MANIFEST_DIR")).join(name));
        cases.push(value);
    }
    for value in cases {
        let error = check(&value).unwrap_err();
        assert!(
            !error.contains("PUBLIC-BURNED-PATH") && !error.contains(env!("CARGO_MANIFEST_DIR"))
        );
    }
    assert!(gate::check_coverage(b"not-json", Path::new(env!("CARGO_MANIFEST_DIR"))).is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("below.json");
    std::fs::write(&path, serde_json::to_vec(&fixture(8999)).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_matrix-evidence"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--coverage")
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}
