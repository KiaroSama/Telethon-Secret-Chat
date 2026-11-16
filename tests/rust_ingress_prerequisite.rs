#![allow(clippy::unwrap_used)]
#[test]
fn unknown_upstream_source_fails_without_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("unknown-source.tar");
    let output = dir.path().join("prepared");
    std::fs::write(&source, b"not the pinned public grammers source").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_prepare-sdk"))
        .arg("--source")
        .arg(source)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("upstream source checksum does not match")
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
