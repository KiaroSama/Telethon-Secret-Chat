use grammers_tl_gen::{Config, Outputs, generate_rust_code};
use grammers_tl_parser::parse_tl_file;
use std::io::Write;
use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
};

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=schema/end-to-end.tl");
    let source = fs::read_to_string("schema/end-to-end.tl")?;
    // Historical constructors share names. Retain their IDs and result families,
    // and suffix only the older Rust names before passing them to the generator.
    let mut counts = HashMap::new();
    for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
        if let Some((name, _)) = line.split_once('#') {
            *counts.entry(name.to_owned()).or_insert(0usize) += 1;
        }
    }
    let mut normalized = String::new();
    for line in source.lines() {
        if !line.trim_start().starts_with("//")
            && let Some((name, rest)) = line.split_once('#')
            && let Some(count) = counts.get_mut(name)
        {
            *count -= 1;
            if *count > 0 {
                let id = rest
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| io::Error::other("Invalid schema ID"))?;
                normalized.push_str(&format!("{name}_legacy{id}#{rest}\n"));
                continue;
            }
        }
        normalized.push_str(line);
        normalized.push('\n');
    }
    let definitions = parse_tl_file(&normalized)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| io::Error::other("The secret schema could not be parsed"))?;
    let directory =
        PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("Missing OUT_DIR"))?);
    let mut outputs = Outputs {
        common: fs::File::create(directory.join("secret_common.rs"))?,
        types: fs::File::create(directory.join("secret_types.rs"))?,
        functions: fs::File::create(directory.join("secret_functions.rs"))?,
        enums: fs::File::create(directory.join("secret_enums.rs"))?,
    };
    for output in [
        &mut outputs.common,
        &mut outputs.types,
        &mut outputs.functions,
        &mut outputs.enums,
    ] {
        writeln!(
            output,
            "// Generated from schema/end-to-end.tl; do not edit."
        )?;
    }
    // Generated Debug is disabled because media constructors contain file keys.
    generate_rust_code(
        &mut outputs,
        &definitions,
        144,
        &Config {
            gen_name_for_id: true,
            impl_debug: false,
            ..Default::default()
        },
    )?;
    outputs.flush()?;
    bundle_sdk(&directory)
}

// Developer tool artifacts must retain their complete inputs after the build
// checkout disappears. Paths are source-relative and files are compile-time bytes.
fn bundle_sdk(output: &Path) -> io::Result<()> {
    let root = Path::new("vendor/grammers");
    let mut files = Vec::new();
    for name in ["grammers-session", "grammers-mtsender", "grammers-client"] {
        sdk_files(&root.join(name), &mut files)?;
    }
    for name in ["README.md", "upstream-hashes.json"] {
        files.push(root.join(name));
    }
    files.sort();
    let mut generated = String::from(
        "// Generated bundled SDK input table; do not edit.\nconst SDK_FILES: &[(&str, &[u8])] = &[\n",
    );
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let relative = file.strip_prefix(root).map_err(io::Error::other)?;
        let relative = relative
            .to_str()
            .ok_or_else(|| io::Error::other("SDK path is not UTF-8"))?
            .replace('\\', "/");
        generated.push_str(&format!("({relative:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/vendor/grammers/\", {relative:?}))),\n"));
    }
    generated.push_str("];\n");
    fs::write(output.join("bundled_sdk.rs"), generated)
}
fn sdk_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Err(io::Error::other("SDK input cannot be a symbolic link"));
    }
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == ".ai" || name == "target" {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("SDK input cannot be a symbolic link"));
        }
        if kind.is_dir() {
            sdk_files(&entry.path(), files)?;
        } else if matches!(
            entry.path().extension().and_then(|part| part.to_str()),
            Some("rs" | "toml" | "md" | "tl")
        ) || matches!(name.to_str(), Some("LICENSE-MIT" | "LICENSE-APACHE"))
        {
            files.push(entry.path());
        }
    }
    Ok(())
}
