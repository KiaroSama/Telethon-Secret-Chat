#![allow(clippy::unwrap_used)]
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};
fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(path).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, files);
        } else if p.extension().is_some_and(|x| {
            ["rlib", "so", "dylib", "dll", "rmeta"].contains(&x.to_string_lossy().as_ref())
        }) {
            files.push(p);
        }
    }
}
/// Compile against the actual built library, checking Rust's stronger typed input
/// boundary rather than introducing a dynamic untyped visualization API.
#[allow(dead_code)] // Called by visualization target; schema target checks a whole program.
pub fn refuses(expression: &str) {
    refuses_program(
        &format!(
            "pub fn check() {{ let _ = kiaro_secret_chat::visualization::key_visualization({expression}); }}"
        ),
        "E0308",
    );
}
pub fn refuses_program(program: &str, diagnostic: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profile = std::env::current_exe()
        .unwrap()
        .ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|x| x == "debug" || x == "release")
        })
        .unwrap()
        .to_owned();
    let mut libraries = Vec::new();
    collect(&profile, &mut libraries);
    let library = libraries
        .iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("libkiaro_secret_chat")
        })
        .max_by_key(|p| std::fs::metadata(p).unwrap().modified().unwrap())
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("boundary.rs");
    std::fs::write(&source, program).unwrap();
    let rustc =
        Path::new(env!("CARGO")).with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" });
    let mut command = Command::new(rustc);
    command
        .current_dir(root)
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("--emit=metadata")
        .arg("--out-dir")
        .arg(directory.path())
        .arg("--extern")
        .arg(format!("kiaro_secret_chat={}", library.display()));
    let parents: BTreeSet<_> = libraries.iter().filter_map(|p| p.parent()).collect();
    for parent in parents {
        command
            .arg("-L")
            .arg(format!("dependency={}", parent.display()));
    }
    let output = command.arg(&source).output().unwrap();
    let errors = String::from_utf8(output.stderr).unwrap();
    assert!(
        !output.status.success(),
        "typed API accepted invalid program"
    );
    assert!(
        errors.contains(diagnostic),
        "compile must fail at input type, not setup: {errors}"
    );
    assert!(
        !errors.contains("E0463"),
        "missing crate is not input refusal: {errors}"
    );
}
