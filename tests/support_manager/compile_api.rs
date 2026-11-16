#![allow(clippy::unwrap_used, clippy::expect_used)]
pub fn compile_api(source: &str) -> std::process::Output {
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
    let executable = std::env::current_exe().unwrap();
    let profile = executable
        .ancestors()
        .find(|p| p.join("build").is_dir() || p.join("deps").is_dir())
        .unwrap();
    static LIBRARY_BUILD: std::sync::OnceLock<std::result::Result<(), String>> =
        std::sync::OnceLock::new();
    let build = LIBRARY_BUILD.get_or_init(|| {
        let mut cargo = std::process::Command::new(
            std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
        );
        cargo.current_dir(env!("CARGO_MANIFEST_DIR")).args([
            "build",
            "--locked",
            "--offline",
            "--lib",
            "-j1",
        ]);
        let profile_name = profile.file_name().unwrap();
        if profile_name == "release" {
            cargo.arg("--release");
        } else if profile_name != "debug" {
            cargo.arg("--profile").arg(profile_name);
        }
        let parent = profile.parent().unwrap();
        let target = parent
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.starts_with(&format!("{}-", std::env::consts::ARCH)));
        if let Some(target) = target {
            cargo
                .arg("--target")
                .arg(target)
                .arg("--target-dir")
                .arg(parent.parent().unwrap());
        } else {
            cargo.arg("--target-dir").arg(parent);
        }
        let output = cargo
            .output()
            .map_err(|error| format!("native metadata build could not start: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "native metadata build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    });
    assert!(
        build.is_ok(),
        "{}",
        build.as_ref().err().map_or("", String::as_str)
    );
    let temporary = tempfile::tempdir_in(profile).unwrap();
    let mut pending = vec![profile.join("build"), profile.join("deps")];
    let mut libraries: BTreeMap<OsString, (std::time::SystemTime, PathBuf)> = BTreeMap::new();
    while let Some(path) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
                continue;
            }
            if !matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("rlib" | "rmeta" | "so" | "dylib" | "dll")
            ) {
                continue;
            }
            let name = entry.file_name();
            let modified = entry.metadata().unwrap().modified().unwrap();
            if libraries
                .get(&name)
                .is_none_or(|(previous, _)| *previous < modified)
            {
                libraries.insert(name, (modified, path));
            }
        }
    }
    let library = libraries
        .iter()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|s| s.starts_with("libkiaro_secret_chat-") && s.ends_with(".rlib"))
        })
        .max_by_key(|(_, entry)| entry.0)
        .map(|(_, entry)| entry.1.clone())
        .expect("cargo build --lib supplies metadata for a focused unit-only API check");
    let dependencies = temporary.path().join("dependencies");
    std::fs::create_dir(&dependencies).unwrap();
    for (name, (_, path)) in libraries {
        std::fs::hard_link(path, dependencies.join(name)).unwrap();
    }
    let file = temporary.path().join("storage_api.rs");
    std::fs::write(&file, source).unwrap();
    std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")))
        .arg("--edition=2024")
        .args([
            "--crate-type=lib",
            "--emit=metadata",
            "--crate-name=strong_storage_api",
        ])
        .arg("--extern")
        .arg(format!("kiaro_secret_chat={}", library.to_str().unwrap()))
        .arg("-L")
        .arg(format!("dependency={}", dependencies.to_str().unwrap()))
        .arg("--out-dir")
        .arg(temporary.path())
        .arg(file)
        .output()
        .unwrap()
}
