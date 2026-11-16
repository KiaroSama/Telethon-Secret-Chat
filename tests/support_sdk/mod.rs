//! Build a test-only SDK copy to replace private network I/O. No installed or
//! production dependency is mutated and no test hook is shipped in the library.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
struct Harness {
    _directory: tempfile::TempDir,
    binary: PathBuf,
}
static HARNESS: OnceLock<Harness> = OnceLock::new();
fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let name = entry.file_name();
        if name == ".ai" || name == "target" {
            continue;
        }
        assert!(
            !kind.is_symlink(),
            "SDK fixture source cannot contain a symbolic link"
        );
        let target = destination.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)
        } else {
            let path = entry.path();
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            let approved = matches!(extension, "rs" | "tl" | "png")
                || matches!(
                    name.to_str(),
                    Some(
                        "Cargo.toml"
                            | "README.md"
                            | "LICENSE-MIT"
                            | "LICENSE-APACHE"
                            | "upstream-hashes.json"
                            | "local-patches.json"
                    )
                );
            if approved {
                std::fs::copy(path, target).unwrap();
            }
        }
    }
}
fn build() -> Harness {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = tempfile::tempdir().unwrap();
    let stage = directory.path().join("source");
    std::fs::create_dir(&stage).unwrap();
    for name in ["src", "schema", "vendor/grammers"] {
        copy_tree(&root.join(name), &stage.join(name));
    }
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "README.md",
        "LICENSE",
        "NOTICE",
    ] {
        std::fs::copy(root.join(name), stage.join(name)).unwrap();
    }
    let sdk = stage.join("vendor/grammers/grammers-mtsender");
    let path = sdk.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path).unwrap();
    assert_eq!(manifest.matches("[features]").count(), 1);
    std::fs::write(
        path,
        manifest.replacen(
            "[features]",
            "[features]\nkiaro-offline-test-fixture = []",
            1,
        ),
    )
    .unwrap();
    let path = sdk.join("src/sender_pool.rs");
    let mut source = std::fs::read_to_string(&path).unwrap();
    source.push('\n');
    source.push_str(include_str!(
        "../fixtures/sdk_ingress_harness/runner_extension.rs"
    ));
    std::fs::write(path, source).unwrap();
    let fixture = directory.path().join("harness");
    copy_tree(
        &root.join("tests/fixtures/sdk_ingress_harness/src"),
        &fixture.join("src"),
    );
    let quote =
        |path: &Path| serde_json::to_string(path.to_str().expect("UTF-8 fixture path")).unwrap();
    let manifest = format!(
        "[package]\nname='kiaro-sdk-ingress-harness'\nversion='0.0.0'\nedition='2024'\n[dependencies]\nkiaro-secret-chat={{path={}}}\ngrammers-mtsender={{path={},features=['kiaro-offline-test-fixture']}}\ntokio={{version='1.54.1',features=['rt','macros','sync','time']}}\nsha1='0.11.0'\ntempfile='3'\nserde_json='1'\n[workspace]\n",
        quote(&stage),
        quote(&sdk)
    );
    std::fs::write(fixture.join("Cargo.toml"), manifest).unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let mut build = Command::new(env!("CARGO"));
    build
        .args([
            "build",
            "--offline",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(fixture.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env("SDK_FIXTURE_ORIGIN", root);
    if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        let mut flags = std::env::var("CARGO_ENCODED_RUSTFLAGS")
            .map(|v| v.split('\x1f').map(str::to_owned).collect::<Vec<_>>())
            .unwrap_or_else(|_| {
                std::env::var("RUSTFLAGS")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect()
            });
        flags.push(format!(
            "--remap-path-prefix={}={}",
            stage.display(),
            root.display()
        ));
        // The standalone fixture is outside llvm-cov's workspace target list.
        // Instrument its final executable as well so it links and flushes the
        // profiling runtime for the genuinely exercised dependency code.
        flags.push("-Cinstrument-coverage".into());
        flags.push("--cfg=coverage".into());
        flags.push(format!(
            "--remap-path-prefix={}={}",
            fixture.join("src").display(),
            root.join("tests/fixtures/sdk_ingress_harness/src")
                .display()
        ));
        build.env("CARGO_ENCODED_RUSTFLAGS", flags.join("\x1f"));
    }
    let output = build.output().unwrap();
    assert!(
        output.status.success(),
        "Offline SDK fixture build failed: {}\n{}",
        String::from_utf8(output.stderr).expect("UTF-8 Cargo output"),
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|message| message["reason"] == "compiler-message")
            .filter_map(|message| message["message"]["rendered"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let binary = compiler_executable(&output.stdout)
        .expect("Cargo must report the just-built native SDK fixture executable");
    assert!(binary.is_file());
    Harness {
        _directory: directory,
        binary,
    }
}
pub(crate) fn compiler_executable(output: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(output).ok()?.lines().find_map(|line| {
        let message: serde_json::Value = serde_json::from_str(line).ok()?;
        if message["reason"] != "compiler-artifact"
            || message["target"]["name"] != "kiaro-sdk-ingress-harness"
            || !message["target"]["kind"]
                .as_array()?
                .iter()
                .any(|kind| kind == "bin")
        {
            return None;
        }
        message["executable"].as_str().map(PathBuf::from)
    })
}
pub fn run(scenario: &str) {
    let harness = HARNESS.get_or_init(build);
    let output = Command::new(&harness.binary)
        .arg(scenario)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "SDK scenario failed: {}",
        String::from_utf8(output.stderr).expect("UTF-8 fixture output")
    );
    assert_eq!(
        std::str::from_utf8(&output.stdout).unwrap().trim(),
        if scenario == "adapter" {
            "Native adapter request, encryption and file boundaries verified"
        } else {
            "SDK receipt and deleting-server frontier verified"
        }
    );
}
