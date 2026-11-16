#![allow(clippy::unwrap_used)]
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
    time::SystemTime,
};
struct Artifacts {
    profile: PathBuf,
    library: PathBuf,
    files: Vec<PathBuf>,
    test_executables: Vec<PathBuf>,
    target: OsString,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrebuiltContext {
    version: u32,
    checkout_sha: String,
    manifest_path: PathBuf,
    target: String,
    profile: String,
    target_dir: PathBuf,
    rustflags: String,
    encoded_rustflags: Option<String>,
    compiler: String,
    graph_sha256: String,
}
fn rustc_program() -> OsString {
    std::env::var_os("RUSTC").unwrap_or_else(|| {
        Path::new(env!("CARGO"))
            .with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" })
            .into_os_string()
    })
}
fn compiler_version() -> Option<String> {
    let output = Command::new(rustc_program()).arg("-vV").output().ok()?;
    output.status.success().then_some(())?;
    Some(
        String::from_utf8(output.stdout)
            .ok()?
            .replace("\r\n", "\n")
            .trim_end()
            .to_owned(),
    )
}
fn git_output(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}
fn fresh_file(path: &Path, receipt_time: SystemTime) -> Option<()> {
    let metadata = fs::metadata(path).ok()?;
    (metadata.is_file() && metadata.modified().ok()? <= receipt_time).then_some(())
}
fn fresh_source(root: &Path, receipt_time: SystemTime, expected: &str) -> Option<()> {
    let head = String::from_utf8(git_output(root, &["rev-parse", "HEAD"])?).ok()?;
    (head.trim() == expected).then_some(())?;
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        (sha == expected).then_some(())?;
    }
    let inputs = git_output(root, &["ls-files", "--cached", "-z"])?;
    for name in inputs
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let path = Path::new(std::str::from_utf8(name).ok()?);
        fresh_file(&root.join(path), receipt_time)?;
    }
    // Unrelated logs and ignored build outputs are not compilation inputs.
    let others = git_output(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for name in others
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let path = Path::new(std::str::from_utf8(name).ok()?);
        if path.starts_with("src") || path.starts_with("schema") || path.starts_with("vendor") {
            fresh_file(&root.join(path), receipt_time)?;
        }
    }
    Some(())
}
fn prebuilt_artifacts(root: &Path, profile: &Path, target: &OsString) -> Option<Artifacts> {
    let graph = PathBuf::from(std::env::var_os("KIARO_COMPILE_ARTIFACTS")?);
    if !graph.is_absolute()
        || [
            "LLVM_PROFILE_FILE",
            "CARGO_LLVM_COV",
            "CARGO_LLVM_COV_TARGET_DIR",
            "__CARGO_LLVM_COV_RUSTC_WRAPPER",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
        || ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"]
            .iter()
            .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
        || profile.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            name.contains("llvm-cov") || name.contains("coverage")
        })
    {
        return None;
    }
    read_prebuilt_artifacts(root, profile, target, &graph)
}
fn read_prebuilt_artifacts(
    root: &Path,
    profile: &Path,
    target: &OsString,
    graph: &Path,
) -> Option<Artifacts> {
    let mut sidecar = graph.as_os_str().to_os_string();
    sidecar.push(".context.json");
    let sidecar = PathBuf::from(sidecar);
    let receipt_time = fs::metadata(&sidecar).ok()?.modified().ok()?;
    let context: PrebuiltContext = serde_json::from_slice(&fs::read(&sidecar).ok()?).ok()?;
    let profile = profile.canonicalize().ok()?;
    let target_dir = context.target_dir.canonicalize().ok()?;
    let current_target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR")?);
    let current_target = if current_target.is_absolute() {
        current_target
    } else {
        root.join(current_target)
    };
    let compiler = compiler_version()?;
    let encoded = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .ok()
        .filter(|value| !value.is_empty());
    if context.version != 1
        || !context.manifest_path.is_absolute()
        || context.manifest_path.canonicalize().ok()?
            != root.join("Cargo.toml").canonicalize().ok()?
        || context.target != target.to_str()?
        || !matches!(context.profile.as_str(), "debug" | "release")
        || profile.file_name()?.to_str()? != context.profile
        || target_dir != profile.parent()?
        || current_target.canonicalize().ok()? != target_dir
        || context.rustflags != std::env::var("RUSTFLAGS").unwrap_or_default()
        || context.encoded_rustflags.filter(|value| !value.is_empty()) != encoded
        || context.compiler.replace("\r\n", "\n").trim_end() != compiler
        || !compiler
            .lines()
            .any(|line| line.strip_prefix("host: ") == Some(context.target.as_str()))
    {
        return None;
    }
    fresh_file(graph, receipt_time)?;
    fresh_source(root, receipt_time, &context.checkout_sha)?;
    let bytes = fs::read(graph).ok()?;
    (hex::encode(Sha256::digest(&bytes)) == context.graph_sha256).then_some(())?;
    let artifacts = collect_artifacts(root, profile, target.clone(), &bytes).ok()?;
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    artifacts
        .test_executables
        .iter()
        .any(|path| path.canonicalize().ok().as_ref() == Some(&executable))
        .then_some(())?;
    fresh_file(&executable, receipt_time)?;
    for path in &artifacts.files {
        path.canonicalize()
            .ok()?
            .starts_with(&target_dir)
            .then_some(())?;
        fresh_file(path, receipt_time)?;
    }
    artifacts
        .library
        .canonicalize()
        .ok()?
        .starts_with(&artifacts.profile)
        .then_some(())?;
    Some(artifacts)
}
/// Additional cache guards run inside an existing native type-boundary case.
pub fn prebuilt_receipt_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let executable = std::env::current_exe().unwrap();
    let profile = executable
        .ancestors()
        .find(|path| path.join("build").is_dir() || path.join("deps").is_dir())
        .unwrap();
    let target = OsString::from(env!("KIARO_COMPILE_TARGET"));
    let Some(accepted) = prebuilt_artifacts(root, profile, &target) else {
        // Non-receipt and instrumented cohorts retain the original real Cargo
        // build and the same positive/negative consumer assertions below.
        return;
    };
    let original = PathBuf::from(std::env::var_os("KIARO_COMPILE_ARTIFACTS").unwrap());
    let bytes = fs::read(&original).unwrap();
    let mut sidecar = original.as_os_str().to_os_string();
    sidecar.push(".context.json");
    let context: serde_json::Value = serde_json::from_slice(&fs::read(sidecar).unwrap()).unwrap();
    let directory = tempfile::tempdir_in(profile).unwrap();
    let graph = directory.path().join("receipt.jsonl");
    let sidecar = directory.path().join("receipt.jsonl.context.json");
    let write = |bytes: &[u8], mut context: serde_json::Value| {
        fs::write(&graph, bytes).unwrap();
        context["graph_sha256"] = hex::encode(Sha256::digest(bytes)).into();
        fs::write(&sidecar, serde_json::to_vec(&context).unwrap()).unwrap();
    };
    let accepted_receipt = || read_prebuilt_artifacts(root, profile, &target, &graph).is_some();
    write(&bytes, context.clone());
    assert!(
        accepted_receipt(),
        "a fresh real prebuild graph must be accepted"
    );
    for (field, invalid) in [
        (
            "manifest_path",
            root.join("wrong-manifest.toml").display().to_string(),
        ),
        (
            "profile",
            if context["profile"] == "release" {
                "debug"
            } else {
                "release"
            }
            .into(),
        ),
        ("checkout_sha", "0".repeat(40)),
        ("target", "wrong-target".into()),
        ("compiler", "wrong compiler".into()),
    ] {
        let mut wrong = context.clone();
        wrong[field] = invalid.into();
        write(&bytes, wrong);
        assert!(!accepted_receipt(), "receipt must reject invalid {field}");
    }
    write(&bytes, context.clone());
    fs::OpenOptions::new()
        .write(true)
        .open(&sidecar)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    assert!(
        !accepted_receipt(),
        "an old receipt cannot attest current inputs"
    );
    assert!(
        fresh_source(
            root,
            SystemTime::UNIX_EPOCH,
            context["checkout_sha"].as_str().unwrap()
        )
        .is_none()
    );
    let records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let encode = |records: &[serde_json::Value]| {
        let mut output = Vec::new();
        for record in records {
            serde_json::to_writer(&mut output, record).unwrap();
            output.push(b'\n');
        }
        output
    };
    let mut failed = records.clone();
    assert_eq!(failed.last().unwrap()["reason"], "build-finished");
    failed.last_mut().unwrap()["success"] = false.into();
    write(&encode(&failed), context.clone());
    assert!(
        !accepted_receipt(),
        "a failed final build marker is not a receipt"
    );
    failed.pop();
    write(&encode(&failed), context.clone());
    assert!(!accepted_receipt(), "a partial graph is not a receipt");
    let mut unrelated = records.clone();
    for record in &mut unrelated {
        if record["reason"] == "compiler-artifact" && record["profile"]["test"] == true {
            record["executable"] = serde_json::Value::Null;
        }
    }
    write(&encode(&unrelated), context.clone());
    assert!(
        !accepted_receipt(),
        "the running test executable must belong to this exact graph"
    );
    let mut test_only = records.clone();
    for record in &mut test_only {
        if record["target"]["name"] == "kiaro_secret_chat"
            && record["target"]["kind"] == serde_json::json!(["lib"])
        {
            record["profile"]["test"] = true.into();
        }
    }
    write(&encode(&test_only), context.clone());
    assert!(
        !accepted_receipt(),
        "a test-harness root cannot replace the ordinary library"
    );
    let source = accepted
        .files
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "rmeta")
        })
        .unwrap_or(&accepted.library);
    let newer = directory.path().join(source.file_name().unwrap());
    fs::copy(source, &newer).unwrap();
    let mut changed = records;
    let mut replacements = 0;
    for record in &mut changed {
        if let Some(files) = record["filenames"].as_array_mut() {
            for file in files {
                if file.as_str().is_some_and(|file| Path::new(file) == source) {
                    *file = newer.to_str().unwrap().into();
                    replacements += 1;
                }
            }
        }
    }
    assert!(replacements > 0);
    write(&encode(&changed), context);
    fs::OpenOptions::new()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(SystemTime::now() + std::time::Duration::from_secs(60)),
        )
        .unwrap();
    assert!(
        !accepted_receipt(),
        "a newer graph artifact must invalidate the receipt"
    );
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
    if let Some(artifacts) = prebuilt_artifacts(root, &profile, &target) {
        return Ok(artifacts);
    }
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
    collect_artifacts(root, profile, target, &output.stdout)
}
fn collect_artifacts(
    root: &Path,
    profile: PathBuf,
    target: OsString,
    messages: &[u8],
) -> Result<Artifacts, String> {
    let manifest = root
        .join("Cargo.toml")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    let mut library = None;
    let mut test_executables = Vec::new();
    let mut finished = false;
    let mut finished_count = 0;
    for line in messages
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let artifact: serde_json::Value = serde_json::from_slice(line)
            .map_err(|e| format!("invalid Cargo artifact message: {e}"))?;
        finished = artifact["reason"] == "build-finished" && artifact["success"] == true;
        if artifact["reason"] == "build-finished" {
            finished_count += 1;
        }
        if artifact["reason"] != "compiler-artifact" {
            continue;
        }
        let root_package = artifact["manifest_path"]
            .as_str()
            .map(Path::new)
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            == Some(&manifest);
        if root_package
            && artifact["profile"]["test"] == true
            && let Some(executable) = artifact["executable"].as_str()
        {
            test_executables.push(PathBuf::from(executable));
        }
        let root_library = root_package
            && artifact["target"]["name"] == "kiaro_secret_chat"
            && artifact["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
            && artifact["profile"]["test"] == false;
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
    if !finished || finished_count != 1 {
        return Err("Cargo artifact graph has no successful final build marker".into());
    }
    Ok(Artifacts {
        profile,
        library: library.ok_or("Cargo did not report the ordinary root library artifact")?,
        files: files.into_values().collect(),
        test_executables,
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
    let mut command = Command::new(rustc_program());
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
