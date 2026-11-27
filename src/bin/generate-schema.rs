#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut schema_path = PathBuf::from("schema/end-to-end.tl");
    let mut out_path = PathBuf::from("schema/secret_tl.generated.json");
    let mut check_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check_only = true,
            "--schema" => {
                if let Some(p) = args.next() {
                    schema_path = PathBuf::from(p);
                } else {
                    eprintln!("ERROR: --schema requires a path");
                    return ExitCode::from(2);
                }
            }
            "--out" => {
                if let Some(p) = args.next() {
                    out_path = PathBuf::from(p);
                } else {
                    eprintln!("ERROR: --out requires a path");
                    return ExitCode::from(2);
                }
            }
            other => {
                eprintln!("ERROR: unrecognized argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let source = match std::fs::read_to_string(&schema_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("ERROR: cannot read schema {}: {err}", schema_path.display());
            return ExitCode::from(1);
        }
    };

    let inspected = match kiaro_secret_chat::ported::tool_generate_schema::inspect_schema(source) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("ERROR: schema parse failed: {err}");
            return ExitCode::from(1);
        }
    };

    let ctor_count = inspected["count"].as_u64().unwrap_or(0);
    if ctor_count == 0 {
        eprintln!("ERROR: No constructors parsed; the tracked schema may be incomplete");
        return ExitCode::from(1);
    }

    let mut rendered = match serde_json::to_string_pretty(&inspected) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("ERROR: failed to serialize schema output: {err}");
            return ExitCode::from(1);
        }
    };
    rendered.push('\n');

    if check_only {
        let existing = std::fs::read_to_string(&out_path).unwrap_or_default();
        if existing == rendered {
            println!(
                "Schema {} is up to date ({ctor_count} constructors)",
                out_path.display()
            );
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "ERROR: {} is not what the schema generates; regenerate it",
            out_path.display()
        );
        return ExitCode::from(1);
    }

    let Some(parent) = out_path.parent() else {
        eprintln!("ERROR: output path has no parent directory");
        return ExitCode::from(1);
    };
    let mut tmp_name = out_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp_path = parent.join(tmp_name);

    if let Err(err) = std::fs::write(&tmp_path, rendered.as_bytes()) {
        eprintln!("ERROR: failed writing temp schema output: {err}");
        return ExitCode::from(1);
    }
    if out_path.exists() {
        let _ = std::fs::remove_file(&out_path);
    }
    if let Err(err) = std::fs::rename(&tmp_path, &out_path) {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("ERROR: failed replacing schema output: {err}");
        return ExitCode::from(1);
    }
    println!(
        "Generated {} with {ctor_count} constructors",
        out_path.display()
    );
    ExitCode::SUCCESS
}
