#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::Map;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(records_arg) = args.next() else {
        eprintln!("Usage: check-test-matrix RECORDS --expect-legs N LEG [LEG ...]");
        return ExitCode::from(2);
    };
    let records_dir = PathBuf::from(records_arg);
    let mut expect_legs: Option<i64> = None;
    let mut legs: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expect-legs" => {
                let Some(n_str) = args.next() else {
                    eprintln!("ERROR: --expect-legs requires an integer");
                    return ExitCode::from(2);
                };
                match n_str.parse::<i64>() {
                    Ok(n) => expect_legs = Some(n),
                    Err(_) => {
                        eprintln!("ERROR: --expect-legs must be an integer");
                        return ExitCode::from(2);
                    }
                }
            }
            "--debug" => {}
            _ => legs.push(arg),
        }
    }

    let Some(expected_count) = expect_legs else {
        eprintln!("ERROR: --expect-legs is required");
        return ExitCode::from(2);
    };

    let mut artifacts = Map::new();
    match std::fs::read_dir(&records_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    let xml_path = path.join("junit.xml");
                    if let Ok(xml) = std::fs::read_to_string(&xml_path) {
                        artifacts.insert(name.to_owned(), serde_json::Value::String(xml));
                    }
                }
            }
        }
        Err(err) => {
            eprintln!("ERROR: Matrix evidence rejected: {err}");
            return ExitCode::from(1);
        }
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
                        if result["exit"].as_i64() == Some(0) {
                            println!("{text}");
                        } else {
                            eprintln!("{text}");
                        }
                    }
                }
            }
            let code = result["exit"].as_i64().unwrap_or(1);
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        Err(err) => {
            eprintln!("ERROR: Matrix evidence rejected: {err}");
            ExitCode::from(1)
        }
    }
}
