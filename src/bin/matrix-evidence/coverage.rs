//! Verify the same scoped LLVM JSON that is published, including extra objects.
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};
#[derive(Deserialize)]
struct Export {
    #[serde(rename = "type")]
    kind: String,
    version: String,
    data: Vec<Run>,
}
#[derive(Deserialize)]
struct Run {
    files: Vec<File>,
    totals: Metrics,
}
#[derive(Deserialize)]
struct File {
    filename: String,
    summary: Metrics,
}
#[derive(Deserialize)]
struct Metrics {
    lines: Count,
    regions: Count,
    functions: Count,
}
#[derive(Deserialize)]
struct Count {
    count: u64,
    covered: u64,
    percent: f64,
    notcovered: Option<u64>,
}
impl Count {
    fn validate(&self) -> Result<(), String> {
        if self.covered > self.count
            || self
                .notcovered
                .is_some_and(|n| n != self.count - self.covered)
            || !self.percent.is_finite()
            || !(0.0..=100.0).contains(&self.percent)
        {
            return Err("Coverage counts or percentages are invalid".into());
        }
        let actual = if self.count == 0 {
            0.0
        } else {
            100.0 * self.covered as f64 / self.count as f64
        };
        if (actual - self.percent).abs() > 0.000001 {
            return Err("Coverage percentages disagree with measured counts".into());
        }
        Ok(())
    }
}
impl Metrics {
    fn validate(&self) -> Result<(), String> {
        self.lines.validate()?;
        self.regions.validate()?;
        self.functions.validate()
    }
}
fn owned(path: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    !path.starts_with("bin/")
        && !path.starts_with("export/jpeg_decoder/")
        && !path.starts_with("ingress/strong_tests/")
        && !matches!(
            path.as_str(),
            "storage/audit_tests.rs"
                | "storage/contract_tests.rs"
                | "storage/foundation.rs"
                | "storage/guard_tests.rs"
                | "storage/lifecycle_tests.rs"
                | "ingress/attachment_tests.rs"
                | "ingress/contract_tests.rs"
                | "ingress/test_fixture.rs"
                | "ingress/tests.rs"
                | "ingress/strong_tests.rs"
        )
}
pub fn check_coverage(bytes: &[u8], root: &Path) -> Result<String, String> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Coverage summary exceeds its supported size".into());
    }
    let export: Export =
        serde_json::from_slice(bytes).map_err(|_| "Coverage JSON schema is invalid")?;
    let version = export
        .version
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>();
    if export.kind != "llvm.coverage.json.export"
        || !version.is_ok_and(|v| v.len() == 3 && v[0] == 3)
        || export.data.len() != 1
    {
        return Err("Coverage export type, version or run count is unsupported".into());
    }
    let run = &export.data[0];
    run.totals.validate()?;
    if run.files.is_empty() || run.totals.lines.count == 0 {
        return Err("Coverage summary has no measured library lines".into());
    }
    let source = root
        .join("src")
        .canonicalize()
        .map_err(|_| "Coverage must be verified from its source checkout")?;
    let mut names = BTreeSet::new();
    let mut count = 0_u128;
    let mut covered = 0_u128;
    for file in &run.files {
        file.summary.validate()?;
        let normalized = file.filename.replace('\\', "/");
        let path = Path::new(&normalized);
        if !path.is_absolute() {
            return Err("Coverage contains a non-absolute source filename".into());
        }
        let path = path
            .canonicalize()
            .map_err(|_| "Coverage source file is unavailable")?;
        let relative = path
            .strip_prefix(&source)
            .map_err(|_| "Coverage contains a file outside the owned library")?;
        if path.extension().is_none_or(|e| e != "rs") || !owned(relative) {
            return Err("Coverage contains excluded non-library source".into());
        }
        if !names.insert(path) {
            return Err("Coverage contains duplicate source records".into());
        }
        count += u128::from(file.summary.lines.count);
        covered += u128::from(file.summary.lines.covered);
    }
    if count != u128::from(run.totals.lines.count)
        || covered != u128::from(run.totals.lines.covered)
    {
        return Err("Coverage file counts disagree with export totals".into());
    }
    if covered * 100 < count * 90 {
        return Err("Owned library line coverage is below the required 90%".into());
    }
    Ok(format!(
        "Owned library coverage: {covered}/{count} lines ({:.6}%), regions {:.6}%, functions {:.6}%; 90% gate passed",
        run.totals.lines.percent, run.totals.regions.percent, run.totals.functions.percent
    ))
}
