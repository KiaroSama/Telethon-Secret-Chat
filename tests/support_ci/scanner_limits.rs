#![allow(clippy::unwrap_used, dead_code)]
use crate::scanner::process;
pub fn completed_child_remains_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("capture_probe.rs");
    std::fs::write(
        &source,
        r#"use std::io::Write; fn main() {
        let mode=std::env::args().nth(1).unwrap();
        let size=if mode=="small" {32} else {65536};
        let bytes=vec![b'x';size];
        if mode=="stderr" {std::io::stderr().write_all(&bytes).unwrap();}
        else {std::io::stdout().write_all(&bytes).unwrap();}
    }"#,
    )
    .unwrap();
    let binary = dir
        .path()
        .join(format!("capture_probe{}", std::env::consts::EXE_SUFFIX));
    let compiler = std::path::Path::new(env!("CARGO"))
        .with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let output = std::process::Command::new(compiler)
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for mode in ["small", "stdout", "stderr"] {
        let output = process::run(
            std::process::Command::new(&binary).arg(mode),
            dir.path(),
            mode,
            std::time::Duration::from_secs(10),
            4096,
        );
        if mode == "small" {
            assert!(output.unwrap().status.success());
        } else {
            assert!(
                output.is_err(),
                "completed {mode} producer exceeded its bound"
            );
        }
    }
}
