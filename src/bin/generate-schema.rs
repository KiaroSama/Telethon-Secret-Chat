#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

#[path = "common/atomic.rs"]
mod atomic;
mod common;
use common::{Level, log};

const USAGE: &str = "Usage: generate-schema [--schema PATH] [--out PATH] [--check] [--debug]\n\nGenerates a deterministic manifest of the tracked Telegram end-to-end TL constructors.\n--check validates an existing manifest without changing it.";

fn main() -> ExitCode {
    let mut schema_path = PathBuf::from("schema/end-to-end.tl");
    let mut out_path = PathBuf::from("schema/secret_tl.generated.json");
    let mut check_only = false;
    let mut debug = false;

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
                } else {
                    log(Level::Error, "--schema requires a path");
                    return ExitCode::from(2);
                }
            }
            "--out" => {
                if let Some(p) = args.next() {
                    out_path = PathBuf::from(p);
                } else {
                    log(Level::Error, "--out requires a path");
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

    let source = match std::fs::read_to_string(&schema_path) {
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
    let inspected =
        match kiaro_secret_chat::ported::tool_generate_schema::inspect_checked_schema(source) {
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
