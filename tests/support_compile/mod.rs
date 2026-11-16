#![allow(clippy::unwrap_used)]
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
struct Artifacts {
    profile: PathBuf,
    library: PathBuf,
    files: Vec<PathBuf>,
    target: OsString,
}
fn artifacts() -> &'static Artifacts {
    static BUILD: OnceLock<Result<Artifacts, String>> = OnceLock::new();
    BUILD
        .get_or_init(build)
        .as_ref()
        .unwrap_or_else(|error| panic!("native compile-boundary setup failed: {error}"))
}
fn build() -> Result<Artifacts, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let profile = executable
        .ancestors()
        .find(|path| path.join("build").is_dir() || path.join("deps").is_dir())
        .ok_or("the active Cargo profile directory is unavailable")?
        .to_owned();
    let parent = profile
        .parent()
        .ok_or("the active target directory is unavailable")?;
    let target = OsString::from(env!("KIARO_COMPILE_TARGET"));
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into()));
    cargo.current_dir(root).args([
        "build",
        "--locked",
        "--offline",
        "--lib",
        "-j1",
        "--message-format=json",
    ]);
    let name = profile
        .file_name()
        .ok_or("the active profile has no name")?;
    if name == "release" {
        cargo.arg("--release");
    } else if name != "debug" {
        cargo.arg("--profile").arg(name);
    }
    cargo
        .arg("--target")
        .arg(env!("KIARO_COMPILE_TARGET"))
        .arg("--target-dir")
        .arg(parent.join("compile-boundary"));
    // Cargo releases its build lock before running test binaries. Build only the
    // ordinary root library, never tests or a recursive test executable. Flags
    // come from the actual test run. The cache location is merely a directory,
    // never a source of target-triple information.
    let output = cargo.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "cargo build --lib failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let manifest = root
        .join("Cargo.toml")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    let mut library = None;
    for line in output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let artifact: serde_json::Value = serde_json::from_slice(line)
            .map_err(|e| format!("invalid Cargo artifact message: {e}"))?;
        if artifact["reason"] != "compiler-artifact" {
            continue;
        }
        let root_library = artifact["target"]["name"] == "kiaro_secret_chat"
            && artifact["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
            && artifact["manifest_path"]
                .as_str()
                .map(Path::new)
                .and_then(|path| path.canonicalize().ok())
                .as_ref()
                == Some(&manifest);
        for file in artifact["filenames"]
            .as_array()
            .ok_or("Cargo artifact filenames are unavailable")?
        {
            let file = PathBuf::from(file.as_str().ok_or("Cargo artifact filename is invalid")?);
            if !matches!(
                file.extension().and_then(|extension| extension.to_str()),
                Some("rlib" | "rmeta" | "so" | "dylib" | "dll")
            ) {
                continue;
            }
            if root_library
                && file
                    .extension()
                    .is_some_and(|extension| extension == "rlib")
            {
                if library.as_ref().is_some_and(|previous| previous != &file) {
                    return Err("Cargo reported multiple ordinary root libraries".into());
                }
                library = Some(file.clone());
            }
            let name = file
                .file_name()
                .ok_or("Cargo artifact has no filename")?
                .to_owned();
            if let Some(previous) = files.insert(name, file.clone())
                && previous != file
                && std::fs::read(previous).map_err(|e| e.to_string())?
                    != std::fs::read(&file).map_err(|e| e.to_string())?
            {
                return Err("current Cargo graph has conflicting artifact filenames".into());
            }
        }
    }
    Ok(Artifacts {
        profile,
        library: library.ok_or("Cargo did not report the ordinary root library artifact")?,
        files: files.into_values().collect(),
        target,
    })
}
fn compile(
    artifacts: &Artifacts,
    directory: &Path,
    dependencies: &Path,
    name: &str,
    program: &str,
) -> Output {
    let source = directory.join(format!("{name}.rs"));
    std::fs::write(&source, program).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| {
        Path::new(env!("CARGO"))
            .with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" })
            .into_os_string()
    });
    let mut command = Command::new(rustc);
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("--emit=metadata")
        .arg("--crate-name")
        .arg(name)
        .arg("--out-dir")
        .arg(directory)
        .arg("--extern")
        .arg(format!(
            "kiaro_secret_chat={}",
            dependencies
                .join(artifacts.library.file_name().unwrap())
                .display()
        ))
        .arg("-L")
        .arg(format!("dependency={}", dependencies.display()));
    command.arg("--target").arg(&artifacts.target);
    command.arg(source).output().unwrap()
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
    let valid = compile_program(
        r#"
        pub fn check(hash: &[u8], value: kiaro_secret_chat::enums::DecryptedMessage) {
            let _ = kiaro_secret_chat::visualization::key_visualization(hash);
            let key = kiaro_secret_chat::files::FileKey::new(&[0;32], &[0;32]).unwrap();
            let mut input: &[u8] = &[];
            let _ = kiaro_secret_chat::files::save_file(&mut input, std::path::Path::new("never-written.bin"), 0u64, &key, 0);
            let _ = value;
        }
    "#,
    );
    assert!(
        valid.status.success(),
        "valid boundary control must compile before rejecting input: {}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let output = compile_program(program);
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
pub fn compile_program(program: &str) -> Output {
    let artifacts = artifacts();
    let directory = tempfile::tempdir_in(&artifacts.profile).unwrap();
    let dependencies = directory.path().join("dependencies");
    std::fs::create_dir(&dependencies).unwrap();
    // Snapshot only files named by this exact Cargo invocation. Newer rmeta or
    // copied SDK-harness outputs elsewhere in target cannot enter this graph.
    for file in &artifacts.files {
        std::fs::hard_link(file, dependencies.join(file.file_name().unwrap())).unwrap();
    }
    compile(
        artifacts,
        directory.path(),
        &dependencies,
        "boundary_api",
        program,
    )
}
