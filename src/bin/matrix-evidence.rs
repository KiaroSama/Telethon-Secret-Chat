//! Validate executed libtest records, including explicit platform exceptions.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::ExitCode,
};
const LEGS: [&str; 6] = [
    "rust-ubuntu-latest-1.95.0-debug",
    "rust-ubuntu-latest-1.100.0-debug",
    "rust-ubuntu-latest-1.100.0-release",
    "rust-ubuntu-latest-1.100.0-portable-aes",
    "rust-windows-latest-1.100.0-release",
    "rust-macos-latest-1.100.0-release",
];
fn main() -> ExitCode {
    let path = std::env::args().nth(1).unwrap_or_else(|| "records".into());
    match check(Path::new(&path)) {
        Ok(()) => {
            println!(
                "All six Rust matrix records contain the same executed cases and declared skips."
            );
            ExitCode::SUCCESS
        }
        Err(()) => {
            eprintln!("ERROR: Rust matrix evidence is incomplete, failed, or inconsistent.");
            ExitCode::FAILURE
        }
    }
}
fn check(root: &Path) -> Result<(), ()> {
    let actual = fs::read_dir(root)
        .map_err(|_| ())?
        .map(|item| {
            item.map_err(|_| ())
                .and_then(|item| item.file_name().into_string().map_err(|_| ()))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual != LEGS.iter().map(|s| s.to_string()).collect() {
        return Err(());
    }
    let expected: BTreeSet<String> =
        serde_json::from_str(include_str!("../../migration/rust-cases.json")).map_err(|_| ())?;
    if expected.is_empty() {
        return Err(());
    }
    let mut baseline = None;
    for leg in LEGS {
        let text = fs::read_to_string(root.join(leg).join("rust-tests.txt")).map_err(|_| ())?;
        if text.contains("test result: FAILED") || text.contains("error: test failed") {
            return Err(());
        }
        let mut suite = String::new();
        let mut cases = BTreeMap::new();
        let mut summaries = 0;
        for line in text.lines().map(str::trim) {
            if let Some(run) = line.split("Running ").nth(1) {
                suite = run.split(" (").next().ok_or(())?.to_owned();
            }
            if line.starts_with("test result: ok.") {
                summaries += 1;
            }
            if let Some(test) = line.strip_prefix("test ")
                && let Some((name, status)) = test.split_once(" ... ")
            {
                if suite.is_empty() {
                    return Err(());
                }
                let identity = format!("{}::{name}", suite.replace('\\', "/"));
                if cases.insert(identity, status.to_owned()).is_some() {
                    return Err(());
                }
                let unix = matches!(
                    name,
                    "file_state_is_owner_only"
                        | "symlinked_state_is_refused"
                        | "symlinked_lock_is_refused"
                        | "permissive_existing_state_is_refused"
                );
                if status != "ok"
                    && !(leg.contains("windows") && unix && status.starts_with("ignored"))
                {
                    return Err(());
                }
                if leg.contains("windows") && unix && !status.starts_with("ignored") {
                    return Err(());
                }
            }
        }
        if cases.is_empty() || summaries < 4 {
            return Err(());
        }
        let identities = cases.into_keys().collect::<BTreeSet<_>>();
        if identities != expected {
            return Err(());
        }
        if let Some(reference) = &baseline {
            if reference != &identities {
                return Err(());
            }
        } else {
            baseline = Some(identities);
        }
    }
    Ok(())
}
