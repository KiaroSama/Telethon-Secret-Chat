#![allow(clippy::unwrap_used)]
mod tools;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
pub struct Installed {
    pub license: String,
    pub notice: String,
    pub version: String,
    pub declared_version: String,
    pub source: String,
    pub revision: String,
    pub assets: BTreeMap<String, String>,
    pub canonical_sdk: bool,
    pub no_source_stubs: bool,
    pub installed: bool,
    pub lock_unchanged: bool,
    pub manifest_has_vendor: bool,
    pub standalone_tools: bool,
}
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if [".git", ".ai", "target"]
            .iter()
            .any(|x| name == OsStr::new(x))
        {
            continue;
        }
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink(), "source fixtures never follow symlinks");
        let path = entry.path();
        let destination = target.join(name);
        if kind.is_dir() {
            copy_tree(&path, &destination);
        } else {
            let extension = path.extension().and_then(OsStr::to_str).unwrap_or("");
            let name = path.file_name().unwrap().to_string_lossy();
            if matches!(
                extension,
                "rs" | "tl" | "png" | "json" | "md" | "toml" | "lock" | "txt"
            ) || name == "LICENSE"
                || name == "NOTICE"
                || name.starts_with("LICENSE-")
            {
                std::fs::copy(path, destination).unwrap();
            }
        }
    }
}
fn run(command: &mut Command) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "isolated source/Git consumer command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn git(stage: &Path, args: &[&OsStr]) -> std::process::Output {
    run(Command::new("git")
        .current_dir(stage)
        .args([
            "-c",
            "user.name=Native Rust Fixture",
            "-c",
            "user.email=fixture@invalid.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args))
}
fn file_url(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap();
    let path = path
        .to_str()
        .unwrap()
        .strip_prefix("\\\\?\\")
        .unwrap_or(path.to_str().unwrap())
        .replace('\\', "/");
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(&mut encoded, "%{byte:02X}").unwrap();
        }
    }
    format!(
        "file://{}{}",
        if encoded.starts_with('/') { "" } else { "/" },
        encoded
    )
}
fn cargo(manifest: &Path, target: &Path) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(manifest.parent().unwrap())
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_TEST_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0");
    command
}
fn hashes(root: &Path, path: &Path, result: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            hashes(root, &path, result);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            result.insert(
                name,
                hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
            );
        }
    }
}
fn build() -> Installed {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = tempfile::Builder::new()
        .prefix("Rust Git Consumer Space Path ")
        .tempdir()
        .unwrap();
    let stage = directory.path().join("source");
    std::fs::create_dir(&stage).unwrap();
    for folder in [
        "src",
        "schema",
        "vendor/grammers",
        "migration",
        "docs",
        "tools/secret-scan",
    ] {
        copy_tree(&root.join(folder), &stage.join(folder));
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
    let original_lock = std::fs::read(stage.join("Cargo.lock")).unwrap();
    let manifest_text = std::fs::read_to_string(stage.join("Cargo.toml")).unwrap();
    assert!(manifest_text.contains("publish = false"));
    let no_source_stubs = super::source_files::walk(&stage.join("src"))
        .into_iter()
        .all(|path| {
            struct Stub(bool);
            impl<'a> syn::visit::Visit<'a> for Stub {
                fn visit_macro(&mut self, node: &'a syn::Macro) {
                    if node.path.is_ident("todo") || node.path.is_ident("unimplemented") {
                        self.0 = true;
                    }
                    syn::visit::visit_macro(self, node);
                }
            }
            let tree = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
            let mut stub = Stub(false);
            syn::visit::Visit::visit_file(&mut stub, &tree);
            !stub.0
        });
    git(&stage, &[OsStr::new("init"), OsStr::new("--quiet")]);
    git(&stage, &[OsStr::new("add"), OsStr::new(".")]);
    git(
        &stage,
        &[
            OsStr::new("commit"),
            OsStr::new("--quiet"),
            OsStr::new("-m"),
            OsStr::new("Isolated native source fixture"),
        ],
    );
    let revision =
        String::from_utf8(git(&stage, &[OsStr::new("rev-parse"), OsStr::new("HEAD")]).stdout)
            .unwrap()
            .trim()
            .to_owned();
    let tracked = String::from_utf8(git(&stage, &[OsStr::new("ls-files")]).stdout).unwrap();
    assert!(!tracked.contains(".ai/") && !tracked.contains(".session") && !tracked.contains(".db"));
    let source = file_url(&stage);
    let consumer = directory.path().join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(consumer.join("src/main.rs"), include_str!("consumer.rs")).unwrap();
    let quoted = serde_json::to_string(&source).unwrap();
    std::fs::write(consumer.join("Cargo.toml"),format!("[package]\nname='native-source-consumer'\nversion='0.0.0'\nedition='2024'\npublish=false\n[dependencies]\nkiaro-secret-chat={{git={quoted},rev='{revision}'}}\n[workspace]\n")).unwrap();
    std::fs::write(consumer.join("Cargo.lock"), &original_lock).unwrap();
    let manifest = consumer.join("Cargo.toml");
    let target = directory.path().join("isolated target");
    let output = run(cargo(&manifest, &target)
        .args(["metadata", "--format-version", "1"])
        .env("GIT_ALLOW_PROTOCOL", "file"));
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        PathBuf::from(metadata["target_directory"].as_str().unwrap()),
        target,
        "the consumer must override any outer Cargo target directory"
    );
    let packages = metadata["packages"].as_array().unwrap();
    let package = packages
        .iter()
        .find(|p| p["name"] == "kiaro-secret-chat")
        .unwrap();
    let actual_source = package["source"].as_str().unwrap().to_owned();
    assert!(actual_source.starts_with("git+file://") && actual_source.ends_with(&revision));
    let cached = Path::new(package["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap();
    let license = std::fs::read_to_string(cached.join("LICENSE")).unwrap();
    let notice = std::fs::read_to_string(cached.join("NOTICE")).unwrap();
    assert_eq!(
        std::fs::read(cached.join("Cargo.lock")).unwrap(),
        original_lock
    );
    let canonical_sdk = [
        "grammers-client",
        "grammers-session",
        "grammers-mtsender",
        "grammers-tl-types",
    ]
    .iter()
    .all(|name| {
        let packages = packages
            .iter()
            .filter(|p| p["name"] == *name)
            .collect::<Vec<_>>();
        packages.len() == 1 && packages[0]["version"] == "0.12.0"
    });
    let locked = std::fs::read(consumer.join("Cargo.lock")).unwrap();
    let installed_root = directory.path().join("installed");
    run(cargo(&manifest, &target)
        .args(["install", "--path"])
        .arg(&consumer)
        .args(["--offline", "--locked", "--debug", "-j", "1", "--root"])
        .arg(&installed_root));
    let lock_unchanged = std::fs::read(consumer.join("Cargo.lock")).unwrap() == locked
        && std::fs::read(stage.join("Cargo.lock")).unwrap() == original_lock;
    let executable = installed_root.join("bin").join(if cfg!(windows) {
        "native-source-consumer.exe"
    } else {
        "native-source-consumer"
    });
    let empty = directory.path().join("outside checkout");
    std::fs::create_dir(&empty).unwrap();
    let destination = empty.join("export");
    let output = run(Command::new(&executable)
        .current_dir(&empty)
        .arg(&destination));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("public-api\timplemented"));
    let version = stdout
        .lines()
        .find_map(|line| line.strip_prefix("version\t"))
        .unwrap()
        .to_owned();
    let mut assets = BTreeMap::new();
    hashes(&destination, &destination, &mut assets);
    let upstream = Path::new(
        packages
            .iter()
            .find(|p| p["name"] == "grammers-crypto")
            .unwrap()["manifest_path"]
            .as_str()
            .unwrap(),
    )
    .parent()
    .unwrap()
    .parent()
    .unwrap()
    .canonicalize()
    .unwrap();
    let standalone_tools = tools::verify(&stage, &target, &empty, &upstream);
    Installed {
        license,
        notice,
        version,
        declared_version: package["version"].as_str().unwrap().to_owned(),
        source: actual_source,
        revision,
        assets,
        canonical_sdk,
        no_source_stubs,
        installed: executable.is_file() && installed_root.join(".crates2.json").is_file(),
        lock_unchanged,
        manifest_has_vendor: manifest_text.contains("vendor/grammers/*/src/**")
            && manifest_text.contains("docs/rust-*.md"),
        standalone_tools,
    }
}
pub fn installed() -> &'static Installed {
    static INSTALLED: OnceLock<Installed> = OnceLock::new();
    INSTALLED.get_or_init(build)
}
