//! Only bounded location metadata is ever published; reports and process output stay private.
use super::process::Result;
use serde_json::Value;
use std::{fs::OpenOptions, io::Write, path::Path};
pub fn safe_cell(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#x27;"),
            '|' => output.push_str("&#124;"),
            '\r' | '\n' => output.push(' '),
            _ => output.push(ch),
        }
    }
    output
}
pub fn validate_report(bytes: &[u8], code: i32) -> Result<Vec<Value>> {
    if !matches!(code, 0 | 2) {
        return Err("Gitleaks did not complete with a recognized result.");
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "The scanner report format is invalid.")?;
    let findings = value
        .as_array()
        .ok_or("The scanner report is not an array.")?;
    if !findings.iter().all(Value::is_object) {
        return Err("The scanner report contains invalid finding records.");
    }
    if (code == 2) != !findings.is_empty() {
        return Err("The scanner status and report disagree.");
    }
    for finding in findings {
        for key in ["RuleID", "File", "Commit"] {
            if finding
                .get(key)
                .is_some_and(|value| !value.is_string() && !value.is_null())
            {
                return Err("The scanner location metadata has an invalid type.");
            }
        }
        if finding
            .get("StartLine")
            .is_some_and(|value| !value.is_u64() && !value.is_null())
        {
            return Err("The scanner line metadata has an invalid type.");
        }
    }
    Ok(findings.clone())
}
pub fn summary(findings: &[Value], scope: &str) -> String {
    let mut lines=vec!["## Gitleaks".to_owned(),String::new(),format!("Scanner: **{}**",super::VERSION),String::new(),format!("Scope: **{scope}**"),String::new(),"Rules: Pinned built-in Gitleaks rules with standard exclusions and four exact public-fixture/source-checksum exemptions. Full history describes revision coverage, not universal byte or credential coverage.".into(),String::new()];
    if findings.is_empty() {
        lines.push("No matching secrets were found in the scanned Git history.".into());
    } else {
        lines.extend([format!("**{} potential secrets detected.** Review the locations below; rotate any real exposed credentials.",findings.len()),String::new(),"Secret values, source snippets, and commit messages are never included in this summary.".into(),String::new(),"| Rule | File | Line | Commit |".into(),"| --- | --- | --- | --- |".into()]);
        for finding in findings.iter().take(200) {
            let text = |key| finding.get(key).and_then(Value::as_str).unwrap_or("");
            let line = finding
                .get("StartLine")
                .and_then(Value::as_u64)
                .map(|v| v.to_string())
                .unwrap_or_default();
            let cells = [text("RuleID"), text("File"), line.as_str(), text("Commit")];
            lines.push(format!(
                "| {} |",
                cells
                    .iter()
                    .map(|value| format!("<code>{}</code>", safe_cell(value)))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        if findings.len() > 200 {
            lines.extend([String::new(),format!("Showing 200 of {} findings. Repeat the same version locally for the complete report.",findings.len())]);
        }
    }
    lines.join("\n") + "\n"
}
pub fn write(path: &Path, findings: &[Value], scope: &str) -> Result<()> {
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| "The redacted scan summary could not be opened.")?;
    output
        .write_all(summary(findings, scope).as_bytes())
        .map_err(|_| "The redacted scan summary could not be written.")
}
