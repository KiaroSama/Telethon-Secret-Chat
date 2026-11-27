#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::Map;

mod common;
use common::{Level, log};

const USAGE: &str = "Usage: check-test-matrix RECORDS --expect-legs N LEG [LEG ...] [--debug]\n\nValidates archived Python-baseline JUnit evidence and its declared live-test skips.\nThe current Rust CI matrix uses Cargo tests and does not produce this legacy evidence.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(records_arg) = args.next() else {
        log(Level::Error, USAGE);
        return ExitCode::from(2);
    };
    match records_arg.as_str() {
        "--help" | "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        "--version" | "-V" => {
            println!("check-test-matrix {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ if records_arg.starts_with('-') => {
            log(
                Level::Error,
                "Unrecognized argument; use --help for supported options",
            );
            return ExitCode::from(2);
        }
        _ => {}
    }
    let records_dir = PathBuf::from(records_arg);
    let mut expect_legs: Option<i64> = None;
    let mut legs: Vec<String> = Vec::new();
    let mut debug = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expect-legs" => {
                if expect_legs.is_some() {
                    log(Level::Error, "--expect-legs must be specified exactly once");
                    return ExitCode::from(2);
                }
                let Some(n_str) = args.next() else {
                    log(Level::Error, "--expect-legs requires a positive integer");
                    return ExitCode::from(2);
                };
                match n_str.parse::<i64>() {
                    Ok(n) if n > 0 => expect_legs = Some(n),
                    _ => {
                        log(Level::Error, "--expect-legs must be a positive integer");
                        return ExitCode::from(2);
                    }
                }
            }
            "--debug" => debug = true,
            _ if arg.starts_with('-') => {
                log(
                    Level::Error,
                    "Unrecognized argument; use --help for supported options",
                );
                return ExitCode::from(2);
            }
            _ => legs.push(arg),
        }
    }

    let Some(expected_count) = expect_legs else {
        log(Level::Error, "--expect-legs is required");
        return ExitCode::from(2);
    };
    if legs.is_empty() {
        log(Level::Error, "At least one matrix leg name is required");
        return ExitCode::from(2);
    }

    let mut artifacts = Map::new();
    let collected = (|| -> Result<(), std::io::Error> {
        match std::fs::read_dir(&records_dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    let path = entry.path();
                    if kind.is_symlink() {
                        return Err(std::io::Error::other(
                            "artifact paths must not be symbolic links",
                        ));
                    }
                    if kind.is_dir() {
                        let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
                            std::io::Error::other("artifact name is not valid UTF-8")
                        })?;
                        let xml_path = path.join("junit.xml");
                        if !std::fs::symlink_metadata(&xml_path)?.file_type().is_file() {
                            return Err(std::io::Error::other(
                                "artifact evidence must be a regular file",
                            ));
                        }
                        let xml = std::fs::read_to_string(&xml_path)?;
                        artifacts.insert(name.to_owned(), serde_json::Value::String(xml));
                    }
                }
            }
            Err(err) => {
                return Err(err);
            }
        }
        Ok(())
    })();
    if let Err(err) = collected {
        log(
            Level::Error,
            format_args!("Matrix evidence could not be read completely: {err}"),
        );
        return ExitCode::FAILURE;
    }
    if debug {
        log(
            Level::Debug,
            format_args!("Read {} artifact directories", artifacts.len()),
        );
    }

    match kiaro_secret_chat::ported::tool_check_test_matrix::validate(
        serde_json::Value::Object(artifacts),
        legs,
        expected_count,
    ) {
        Ok(result) => {
            if let Some(lines) = result["output"].as_array() {
                for line in lines {
                    if let Some(text) = line.as_str() {
                        if let Some(message) = text.strip_prefix("WARNING: ") {
                            log(Level::Warning, message);
                        } else if let Some(message) = text.strip_prefix("INFO: ") {
                            log(Level::Info, message);
                        } else {
                            log(Level::Error, text.strip_prefix("ERROR: ").unwrap_or(text));
                        }
                    }
                }
            }
            let code = result["exit"].as_i64().unwrap_or(1);
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        Err(err) => {
            log(
                Level::Error,
                format_args!("Matrix evidence rejected: {err}"),
            );
            ExitCode::from(1)
        }
    }
}
