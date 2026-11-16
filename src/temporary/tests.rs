#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_support::logging;
const PREFIX: &str = ".secret-chat-file-";
#[test]
fn living_other_process_is_preserved_and_exit_grants_cleanup() {
    #[cfg(unix)]
    let mut command = std::process::Command::new("/bin/cat");
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("cmd.exe");
        command.args(["/D", "/C", "more"]);
        command
    };
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{PREFIX}{}-owned.tmp", child.id()));
    std::fs::write(&path, b"synthetic plaintext").unwrap();
    assert!(native_process(child.id()) == ProcessState::Alive);
    cleanup(dir.path(), PREFIX);
    assert!(path.exists());
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    assert!(native_process(child.id()) == ProcessState::Dead);
    cleanup(dir.path(), PREFIX);
    assert!(!path.exists());
}
#[test]
fn same_process_temporary_is_registered_until_drop() {
    let dir = tempfile::tempdir().unwrap();
    let mut file = create(dir.path(), PREFIX).unwrap();
    file.write_all(b"owned").unwrap();
    let path = file.path().to_path_buf();
    assert!(ACTIVE.lock().unwrap().contains(&path));
    cleanup(dir.path(), PREFIX);
    assert!(path.exists());
    drop(file);
    assert!(!ACTIVE.lock().unwrap().contains(&path));
    cleanup(dir.path(), PREFIX);
    assert!(!path.exists());
}
#[test]
fn legacy_unknown_and_unrelated_names_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        format!("{PREFIX}legacy.tmp"),
        format!("{PREFIX}4294967296-x.tmp"),
        ".unrelated.tmp".into(),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"untouched").unwrap();
        cleanup(dir.path(), PREFIX);
        assert_eq!(std::fs::read(path).unwrap(), b"untouched");
    }
}
#[test]
fn unqueryable_owner_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("{PREFIX}{}-owned.tmp", std::process::id() + 100000));
    std::fs::write(&path, b"untouched").unwrap();
    cleanup_with(
        dir.path(),
        PREFIX,
        |_| ProcessState::Unknown,
        |path| std::fs::remove_file(path),
    );
    assert!(path.exists());
}
#[test]
fn cleanup_failure_is_redacted_and_deferred() {
    let capture = logging::Capture::default();
    let logs = std::sync::Arc::clone(&capture.0);
    let _guard = tracing::subscriber::set_default(capture);
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("{PREFIX}{}-inactive.tmp", std::process::id()));
    std::fs::write(&path, b"synthetic").unwrap();
    cleanup_with(
        dir.path(),
        PREFIX,
        |_| ProcessState::Dead,
        |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "secret exception payload",
            ))
        },
    );
    assert!(path.exists());
    assert!(!logs.lock().unwrap().is_empty());
    assert!(
        !logs
            .lock()
            .unwrap()
            .join("\n")
            .contains("secret exception payload")
    );
    cleanup(dir.path(), PREFIX);
    assert!(!path.exists());
}
#[test]
fn pid_outside_signed_native_range_never_grants_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{PREFIX}2147483648-owned.tmp"));
    std::fs::write(&path, b"untouched").unwrap();
    cleanup_with(
        dir.path(),
        PREFIX,
        |_| panic!("out-of-range PID must not reach native process lookup"),
        |path| std::fs::remove_file(path),
    );
    assert!(path.exists());
    assert!(native_process(i32::MAX as u32 + 1) == ProcessState::Unknown);
}
fn uncertain_process(kind: io::ErrorKind) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("{PREFIX}{}-owned.tmp", std::process::id() + 100000));
    std::fs::write(&path, b"untouched").unwrap();
    let probe = |_| {
        let failure = io::Error::new(kind, "synthetic lookup failure");
        process_lookup(Err(failure))
    };
    cleanup_with(dir.path(), PREFIX, probe, |path| std::fs::remove_file(path));
    assert!(path.exists());
}
#[test]
fn permission_denied_does_not_prove_owner_death() {
    uncertain_process(io::ErrorKind::PermissionDenied);
}
#[test]
fn operating_system_error_does_not_prove_owner_death() {
    uncertain_process(io::ErrorKind::Other);
}
#[test]
fn invalid_process_query_does_not_prove_owner_death() {
    uncertain_process(io::ErrorKind::InvalidInput);
}
