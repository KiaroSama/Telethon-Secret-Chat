//! Report migration coverage without converting unimplemented cases into passes.
#![forbid(unsafe_code)]

use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    process::ExitCode,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    baseline_commit: String,
    expected_cases: usize,
    cases: Vec<String>,
    mappings: BTreeMap<String, String>,
}
pub fn check_partitions(
    aggregate: &BTreeMap<String, String>,
    partitions: &[BTreeMap<String, String>],
) -> Result<(), &'static str> {
    let mut union = BTreeMap::new();
    for partition in partitions {
        for (case, target) in partition {
            if union.insert(case.clone(), target.clone()).is_some() {
                return Err("A legacy identity appears in multiple owner ledgers");
            }
        }
    }
    if &union != aggregate {
        return Err("Owner ledgers disagree with the aggregate migration mapping");
    }
    Ok(())
}
fn main() -> ExitCode {
    const USAGE: &str = "Usage: migration-audit [--require-complete]\n\nInspect the archived Python-to-Rust migration ledger.";
    let mut args = std::env::args_os().skip(1);
    let argument = args.next();
    let require_complete = match (
        argument.as_deref().and_then(std::ffi::OsStr::to_str),
        args.next(),
    ) {
        (None, None) if argument.is_none() => false,
        (Some("--require-complete"), None) => true,
        (Some("--help" | "-h"), None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        (Some("--version" | "-V"), None) => {
            println!("migration-audit {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("ERROR: {USAGE}");
            return ExitCode::from(2);
        }
    };
    match audit() {
        Ok(complete) => {
            if complete || !require_complete {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(_) => {
            eprintln!("ERROR: The migration ledger is invalid; no parity result was recorded.");
            ExitCode::FAILURE
        }
    }
}
fn audit() -> Result<bool, Box<dyn std::error::Error>> {
    let ledger: Ledger = serde_json::from_str(include_str!("../../migration/legacy-cases.json"))?;
    let partitions = [
        include_str!("../../migration/core-mappings.json"),
        include_str!("../../migration/manager-mappings.json"),
        include_str!("../../migration/export-mappings.json"),
        include_str!("../../migration/primitive-mappings.json"),
        include_str!("../../migration/rekey-mappings.json"),
        include_str!("../../migration/security-mappings.json"),
        include_str!("../../migration/ci-mappings.json"),
    ]
    .map(serde_json::from_str)
    .into_iter()
    .collect::<Result<Vec<BTreeMap<String, String>>, _>>()?;
    check_partitions(&ledger.mappings, &partitions)?;
    let cases = ledger.cases.iter().collect::<BTreeSet<_>>();
    let rust_cases: BTreeSet<String> =
        serde_json::from_str(include_str!("../../migration/rust-cases.json"))?;
    if cases.len() != ledger.expected_cases
        || cases.len() != ledger.cases.len()
        || ledger.expected_cases != 1851
        || ledger.baseline_commit != "d53c7c21574ac404e01b8c3535b0d0fcfa25a193"
        || ledger.mappings.keys().any(|case| !cases.contains(case))
        || ledger
            .mappings
            .values()
            .any(|name| name.is_empty() || !rust_cases.contains(name))
        || ledger.mappings.values().collect::<BTreeSet<_>>().len() != ledger.mappings.len()
    {
        return Err("Invalid case mapping".into());
    }
    println!("Baseline: {}", ledger.baseline_commit);
    println!("Legacy CI cases: {}", cases.len());
    println!(
        "Individually verified Rust equivalents: {}",
        ledger.mappings.len()
    );
    println!(
        "Cases still requiring parity verification: {}",
        cases.len() - ledger.mappings.len()
    );
    let complete =
        cases.len() == ledger.mappings.len() && rust_cases.len() == ledger.expected_cases;
    if !complete {
        println!("INCOMPLETE: Rust tests passing does not establish migration parity.");
    }
    Ok(complete)
}
