//! Reproduce the bundled SDK supplement from a verified official source checkout.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::ExitCode,
};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Upstream {
    repository: String,
    revision: String,
    sha256: BTreeMap<String, String>,
}
fn verify(source: &Path) -> Result<(), ()> {
    let manifest: Upstream =
        serde_json::from_str(include_str!("../../vendor/grammers/upstream-hashes.json"))
            .map_err(|_| ())?;
    if manifest.repository != "https://codeberg.org/Lonami/grammers"
        || manifest.revision != "02367f9711ca3589a8bbb02e4128aca1631c77c4"
        || manifest.sha256.is_empty()
    {
        return Err(());
    }
    for (name, expected) in manifest.sha256 {
        let path = source.join(name);
        if path.ancestors().any(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
        }) {
            return Err(());
        }
        let bytes = std::fs::read(path).map_err(|_| ())?;
        if hex::encode(Sha256::digest(bytes)) != expected {
            return Err(());
        }
    }
    Ok(())
}
include!(concat!(env!("OUT_DIR"), "/bundled_sdk.rs"));
fn write_bundle(target: &Path) -> Result<(), ()> {
    for (relative, bytes) in SDK_FILES {
        let path = target.join(relative);
        std::fs::create_dir_all(path.parent().ok_or(())?).map_err(|_| ())?;
        std::fs::write(path, bytes).map_err(|_| ())?;
    }
    Ok(())
}
fn run() -> Result<(), (&'static str, u8)> {
    let mut args = std::env::args_os().skip(1);
    let mut source = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--source") => source = args.next().map(PathBuf::from),
            Some("--output") => output = args.next().map(PathBuf::from),
            _ => {
                return Err((
                    "Expected --source OFFICIAL_CHECKOUT --output NEW_DIRECTORY",
                    2,
                ));
            }
        }
    }
    let source = source.ok_or(("The official upstream source is required", 2))?;
    let output = output.ok_or(("A new output directory is required", 2))?;
    verify(&source).map_err(|_| {
        (
            "The upstream source checksum does not match the pinned official revision",
            2,
        )
    })?;
    if output.exists() {
        return Err(("The output directory must not already exist", 2));
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".prepared-grammers-")
        .tempdir_in(parent)
        .map_err(|_| ("The SDK output could not be staged", 1))?;
    write_bundle(stage.path()).map_err(|_| ("The SDK supplement could not be staged", 1))?;
    std::fs::rename(stage.path(), output)
        .map_err(|_| ("The SDK output could not be committed", 1))?;
    println!("Verified official grammers source and prepared the Rust SDK supplement.");
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err((message, code)) => {
            eprintln!("ERROR: {message}.");
            ExitCode::from(code)
        }
    }
}
