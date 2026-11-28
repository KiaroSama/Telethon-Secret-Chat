#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

#[path = "common/atomic.rs"]
mod atomic;
mod common;
use common::{Level, log};

const USAGE: &str = "Usage: generate-schema [--schema PATH] [--out JSON_PATH | --output RUST_PATH] [--check] [--debug]\n\nGenerates a deterministic constructor manifest (--out, the default) or a standalone Rust schema module (--output). Rust generation uses the bundled schema unless --schema is supplied and requires rustfmt on PATH. --check validates existing output without changing it.";

fn main() -> ExitCode {
    let mut schema_path = PathBuf::from("schema/end-to-end.tl");
    let mut out_path = PathBuf::from("schema/secret_tl.generated.json");
    let mut check_only = false;
    let mut debug = false;
    let mut schema_explicit = false;
    let mut manifest_explicit = false;
    let mut rust_output = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("generate-schema {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--debug" => debug = true,
            "--check" => check_only = true,
            "--schema" => {
                if let Some(p) = args.next() {
                    schema_path = PathBuf::from(p);
                    schema_explicit = true;
                } else {
                    log(Level::Error, "--schema requires a path");
                    return ExitCode::from(2);
                }
            }
            "--out" => {
                if let Some(p) = args.next() {
                    out_path = PathBuf::from(p);
                    manifest_explicit = true;
                } else {
                    log(Level::Error, "--out requires a path");
                    return ExitCode::from(2);
                }
            }
            "--output" if rust_output.is_none() => {
                if let Some(p) = args.next() {
                    rust_output = Some(PathBuf::from(p));
                } else {
                    log(Level::Error, "--output requires a path");
                    return ExitCode::from(2);
                }
            }
            _ => {
                log(
                    Level::Error,
                    "Unrecognized argument; use --help for supported options",
                );
                return ExitCode::from(2);
            }
        }
    }

    if manifest_explicit && rust_output.is_some() {
        log(
            Level::Error,
            "Choose either --out for JSON or --output for Rust",
        );
        return ExitCode::from(2);
    }
    let loaded = if rust_output.is_some() && !schema_explicit {
        Ok(include_str!("../../schema/end-to-end.tl").to_owned())
    } else {
        std::fs::read_to_string(&schema_path)
    };
    let source = match loaded {
        Ok(s) => s,
        Err(err) => {
            log(Level::Error, format_args!("Cannot read schema: {err}"));
            return ExitCode::from(1);
        }
    };

    if debug {
        log(
            Level::Debug,
            "Loaded schema input; constructor bodies are not logged",
        );
    }
    let inspected = match kiaro_secret_chat::ported::tool_generate_schema::inspect_checked_schema(
        source.clone(),
    ) {
        Ok(v) => v,
        Err(err) => {
            log(Level::Error, format_args!("Schema parse failed: {err}"));
            return ExitCode::from(1);
        }
    };

    let ctor_count = inspected["count"].as_u64().unwrap_or(0);
    if ctor_count == 0 {
        log(
            Level::Error,
            "No constructors parsed; the schema may be incomplete",
        );
        return ExitCode::from(1);
    }

    if let Some(target) = rust_output {
        return generate_rust(&source, &target, check_only);
    }

    let mut rendered = match serde_json::to_string_pretty(&inspected) {
        Ok(s) => s,
        Err(err) => {
            log(
                Level::Error,
                format_args!("Failed to serialize schema output: {err}"),
            );
            return ExitCode::from(1);
        }
    };
    rendered.push('\n');

    if check_only {
        let existing = match std::fs::read_to_string(&out_path) {
            Ok(existing) => existing,
            Err(err) => {
                log(
                    Level::Error,
                    format_args!("Cannot read the existing manifest: {err}"),
                );
                return ExitCode::FAILURE;
            }
        };
        if existing == rendered {
            log(
                Level::Info,
                format_args!("Schema manifest is up to date ({ctor_count} constructors)"),
            );
            return ExitCode::SUCCESS;
        }
        log(
            Level::Error,
            "The manifest differs from the schema output; regenerate it",
        );
        return ExitCode::from(1);
    }

    if out_path.exists() {
        log(
            Level::Warning,
            "Replacing an existing schema manifest after successful validation",
        );
    }
    if let Err(err) = atomic::write(&out_path, rendered.as_bytes()) {
        log(
            Level::Error,
            format_args!("Failed to atomically replace the schema manifest: {err}"),
        );
        return ExitCode::from(1);
    }
    log(
        Level::Info,
        format_args!("Generated schema manifest with {ctor_count} constructors"),
    );
    ExitCode::SUCCESS
}

fn generate_rust(source: &str, target: &std::path::Path, check: bool) -> ExitCode {
    use std::io::Write;
    let result = (|| {
        let mut input = tempfile::NamedTempFile::new().map_err(|_| ())?;
        input.write_all(source.as_bytes()).map_err(|_| ())?;
        input.flush().map_err(|_| ())?;
        kiaro_secret_chat::schema_generator::Generator::default()
            .generate(input.path(), target, check)
            .map_err(|_| ())
    })();
    match result {
        Ok(true) => {
            log(
                Level::Info,
                if check {
                    "Generated Rust schema is up to date"
                } else {
                    "Generated the standalone Rust schema module"
                },
            );
            ExitCode::SUCCESS
        }
        Ok(false) => {
            log(
                Level::Error,
                "The Rust schema output is absent or differs; regenerate it",
            );
            ExitCode::FAILURE
        }
        Err(()) => {
            log(
                Level::Error,
                "Rust schema generation failed; verify the output directory and rustfmt installation",
            );
            ExitCode::FAILURE
        }
    }
}
