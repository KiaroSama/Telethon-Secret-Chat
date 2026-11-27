#![cfg(unix)]

use std::path::Path;
use std::process::Command;

fn source_check(repo: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/scripts/validate-source.sh"))
        .arg("--repo")
        .arg(repo)
        .arg("--inventory-only")
        .output()
        .expect("run the source distribution inventory gate")
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    dir
}

#[test]
fn source_distribution_rejects_tracked_cache_files_even_with_newlines_in_names() {
    for name in [
        "legacy/__pycache__/module.pyc",
        "target/debug/binary",
        "uv.lock",
        "cache/line\nbreak.pyc",
    ] {
        let dir = repository();
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"generated content").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .arg("add")
                .arg("--")
                .arg(name)
                .status()
                .unwrap()
                .success()
        );
        let result = source_check(dir.path());
        assert_eq!(result.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("Forbidden tracked build/cache path")
        );
    }
}

#[test]
fn source_distribution_rejects_an_incomplete_source_inventory() {
    let dir = repository();
    let result = source_check(dir.path());
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Required source path is not tracked")
    );
}
