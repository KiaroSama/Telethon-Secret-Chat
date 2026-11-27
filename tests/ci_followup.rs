use std::path::Path;
use std::process::Command;

fn generate_schema(schema: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_generate-schema"))
        .arg("--schema")
        .arg(schema)
        .arg("--out")
        .arg(output)
        .output()
        .expect("run the schema maintenance command")
}

#[test]
fn malformed_schema_fields_and_results_do_not_replace_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let schema = dir.path().join("input.tl");
    let output = dir.path().join("manifest.json");
    for source in [
        "message#01 payload: = Message;",
        "message#01 payload:Vector<> = Message;",
        "message#01 flags:# payload:flags.32?string = Message;",
        "message#01 payload:missing.0?string = Message;",
        "message#01 payload:int payload:string = Message;",
        "message#01 = Message;;",
    ] {
        std::fs::write(&schema, source).unwrap();
        std::fs::write(&output, b"previous valid manifest").unwrap();
        let result = generate_schema(&schema, &output);
        assert_eq!(
            result.status.code(),
            Some(1),
            "invalid schema was accepted: {source}"
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"previous valid manifest");
    }
}

#[test]
fn schema_manifest_accepts_supported_flags_vectors_and_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let schema = dir.path().join("input.tl");
    let output = dir.path().join("manifest.json");
    std::fs::write(
        &schema,
        "message#00000001 flags:# silent:flags.31?true data:flags.0?Vector<Vector<ns.Item>> = ns.Message; // comment\n",
    )
    .unwrap();
    let result = generate_schema(&schema, &output);
    assert!(result.status.success(), "{result:?}");
    let actual: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({"count": 1, "names": {"00000001": "Message"}})
    );
}

#[cfg(unix)]
mod source_inventory {
    use super::*;

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(init.success());
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "LICENSE",
            "NOTICE",
            "README.md",
            "schema/end-to-end.tl",
            "migration/019-rust-anything-receipt.json",
            "docs/signing/publisher.json",
            "vendor/grammers/upstream-hashes.json",
            "vendor/grammers/local-patches.json",
            "vendor/grammers/grammers-client/Cargo.toml",
            "vendor/grammers/grammers-mtsender/Cargo.toml",
            "vendor/grammers/grammers-session/Cargo.toml",
        ] {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"source inventory fixture").unwrap();
        }
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".gitignore"),
            dir.path().join(".gitignore"),
        )
        .unwrap();
        track(dir.path());
        dir
    }

    fn track(repo: &Path) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["add", "--force", "--all"])
                .status()
                .unwrap()
                .success()
        );
    }

    fn inspect(repo: &Path) -> std::process::Output {
        Command::new("bash")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/scripts/validate-source.sh"))
            .arg("--repo")
            .arg(repo)
            .arg("--inventory-only")
            .output()
            .unwrap()
    }

    #[test]
    fn private_runtime_material_is_refused_even_when_force_added() {
        for path in [
            ".env.live",
            "accounts/owner.session",
            "accounts/owner.session-journal",
            "certs/publisher.pfx",
            "certs/private.key",
        ] {
            let dir = repository();
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"PRIVATE_FILE_CONTENT_CANARY").unwrap();
            track(dir.path());
            let result = inspect(dir.path());
            assert_eq!(
                result.status.code(),
                Some(1),
                "private file was accepted into the source inventory: {path}"
            );
            for output in [&result.stdout, &result.stderr] {
                assert!(!String::from_utf8_lossy(output).contains("PRIVATE_FILE_CONTENT_CANARY"));
            }
        }
    }

    #[test]
    fn public_environment_templates_remain_distributable() {
        let dir = repository();
        for path in [".env.example", ".env.sample", "config/.env.template"] {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"PUBLIC_SETTING=example\n").unwrap();
        }
        track(dir.path());
        let result = inspect(dir.path());
        assert!(result.status.success(), "{result:?}");
    }
}
