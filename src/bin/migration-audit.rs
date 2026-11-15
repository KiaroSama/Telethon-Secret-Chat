//! Report migration coverage without converting unimplemented cases into passes.
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
fn main() -> ExitCode {
    match audit() {
        Ok(complete) => {
            if complete || !std::env::args().any(|a| a == "--require-complete") {
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
