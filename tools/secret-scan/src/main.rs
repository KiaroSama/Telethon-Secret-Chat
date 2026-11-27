//! Native Gitleaks controller; only scanned data and fixed diagnostics cross this boundary.
#![forbid(unsafe_code)]
#[path = "archive.rs"]
mod archive;
#[path = "config.rs"]
pub mod config;
#[path = "process.rs"]
pub(crate) mod process;
#[path = "report.rs"]
pub mod report;
use process::{PrivateDir, Result};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub const VERSION: &str = "8.30.1";
pub const ARCHIVE_SHA256: &str = "551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb";
const URL: &str = "https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_linux_x64.tar.gz";
enum Level {
    Info,
    Warning,
    Error,
    Debug,
}
fn log(level: Level, message: &str) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let level = match level {
        Level::Info => "INFO",
        Level::Warning => "WARNING",
        Level::Error => "ERROR",
        Level::Debug => "DEBUG",
    };
    println!("{timestamp} [{level}] {message}");
}
fn environment(name: &str) -> Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or("A required scanner environment path is unavailable.")
}
pub fn valid_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn wrapper() -> Result<i32> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let history = arguments.get(2) == Some(&OsStr::new("log").to_owned());
    let real = environment("GITLEAKS_REAL_GIT")?;
    let status = Command::new(real)
        .args(arguments)
        .status()
        .map_err(|_| "The verified Git child could not complete.")?;
    let code = status.code().unwrap_or(-1);
    if history {
        fs::write(environment("GITLEAKS_GIT_STATUS")?, code.to_string())
            .map_err(|_| "The Git-history child status could not be recorded.")?;
        if code != 0 {
            eprintln!("ERROR: The Git-history child did not complete successfully.");
        }
    }
    Ok(code)
}
pub fn verify_child_status(path: &Path) -> Result<()> {
    if fs::read(path).map_err(|_| "The Git-history child did not confirm completion.")? != b"0" {
        return Err("The Git-history child did not complete successfully.");
    }
    Ok(())
}
fn scan() -> Result<i32> {
    if std::env::var("RUNNER_DEBUG").as_deref() == Ok("1") {
        return Err(
            "Disable Actions step debug logging before scanning; no clean result was recorded.",
        );
    }
    let workspace = fs::canonicalize(environment("GITHUB_WORKSPACE")?)
        .map_err(|_| "The complete Git workspace is unavailable.")?;
    let git_directory = workspace.join(".git");
    if !git_directory.is_dir() {
        return Err("The complete Git checkout is unavailable.");
    }
    let event =
        std::env::var("GITHUB_EVENT_NAME").map_err(|_| "The workflow event is unavailable.")?;
    if !matches!(
        event.as_str(),
        "push" | "pull_request" | "schedule" | "workflow_dispatch"
    ) {
        return Err("Unsupported workflow event.");
    }
    log(Level::Info, "Starting the repository secret scan.");
    log(
        Level::Debug,
        "Using checksum-pinned Gitleaks; scanned commit contents are never executed.",
    );
    let directory = PrivateDir::new(&environment("RUNNER_TEMP")?)?;
    let temporary = &directory.0;
    let archive_path = temporary.join("gitleaks.tar.gz");
    let download = process::run(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "60",
                "--max-filesize",
                "67108864",
                "--output",
            ])
            .arg(&archive_path)
            .arg(URL),
        temporary,
        "download",
        Duration::from_secs(70),
        1024 * 1024,
    )?;
    if !download.status.success() {
        return Err("The pinned Gitleaks archive could not be downloaded.");
    }
    let bytes =
        fs::read(&archive_path).map_err(|_| "The pinned scanner archive could not be read.")?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != ARCHIVE_SHA256 {
        return Err("The Gitleaks download checksum does not match the pinned release.");
    }
    let expanded = process::run(
        Command::new("gzip").arg("-dc").arg(&archive_path),
        temporary,
        "archive",
        Duration::from_secs(30),
        128 * 1024 * 1024,
    )?;
    if !expanded.status.success() {
        return Err("The verified scanner archive could not be decoded.");
    }
    let binary = temporary.join("gitleaks");
    archive::extract(&expanded.stdout, &binary)?;
    process::executable(&binary)?;
    let version = process::run(
        Command::new(&binary).arg("version"),
        temporary,
        "version",
        Duration::from_secs(15),
        1024 * 1024,
    )?;
    if !version.status.success()
        || fs::read_to_string(version.stdout)
            .map_err(|_| "The scanner version output is unavailable.")?
            .trim()
            != VERSION
    {
        return Err("The Gitleaks executable version could not be verified.");
    }
    log(
        Level::Info,
        "Verified the scanner version and download checksum.",
    );
    let config = temporary.join("scanner.toml");
    fs::write(&config, config::configuration())
        .map_err(|_| "The pinned scanner configuration could not be written.")?;
    let mut log_options =
        "--full-history --diff-filter=uxdb --no-ext-diff --no-textconv --ignore-submodules=all -m"
            .to_owned();
    let scope = if event == "pull_request" {
        let bytes = fs::read(environment("GITHUB_EVENT_PATH")?)
            .map_err(|_| "The pull request event could not be read.")?;
        let event: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "The pull request event is invalid.")?;
        let base = event["pull_request"]["base"]["sha"]
            .as_str()
            .ok_or("The pull request base revision is invalid.")?;
        let head = event["pull_request"]["head"]["sha"]
            .as_str()
            .ok_or("The pull request head revision is invalid.")?;
        for revision in [base, head] {
            if !valid_revision(revision) {
                return Err("The pull request revision is invalid.");
            }
            let checked = process::run(
                Command::new("git").arg("-C").arg(&git_directory).args([
                    "cat-file",
                    "-e",
                    &format!("{revision}^{{commit}}"),
                ]),
                temporary,
                "revision",
                Duration::from_secs(15),
                1024 * 1024,
            )?;
            if !checked.status.success() {
                return Err("A pull request revision is missing from the full checkout.");
            }
        }
        log_options.push_str(&format!(" {base}..{head}"));
        "Commits introduced by this pull request, including merge diffs"
    } else {
        log_options.push_str(" --all");
        "All fetched branches and tags, including full history and merge diffs"
    };
    let real_git = process::real_git()?;
    let wrapper_path = temporary.join("git");
    fs::copy(
        std::env::current_exe().map_err(|_| "The native Git observer could not be located.")?,
        &wrapper_path,
    )
    .map_err(|_| "The native Git observer could not be copied.")?;
    process::executable(&wrapper_path)?;
    let path = std::env::var_os("PATH").ok_or("The scanner PATH is unavailable.")?;
    let path = std::env::join_paths(
        std::iter::once(temporary.clone()).chain(std::env::split_paths(&path)),
    )
    .map_err(|_| "The private scanner PATH is invalid.")?;
    let status_path = temporary.join("git-status");
    let report_path = temporary.join("findings.json");
    let result = process::run(
        Command::new(&binary)
            .arg("git")
            .arg(&git_directory)
            .arg("--config")
            .arg(&config)
            .arg("--gitleaks-ignore-path")
            .arg(temporary)
            .args([
                "--ignore-gitleaks-allow",
                "--redact=100",
                "--no-banner",
                "--no-color",
                "--log-level=error",
                "--exit-code=2",
                "--report-format=json",
                "--report-path",
            ])
            .arg(&report_path)
            .arg(format!("--log-opts={log_options}"))
            .env("PATH", path)
            .env("GITLEAKS_REAL_GIT", real_git)
            .env("GITLEAKS_GIT_STATUS", &status_path),
        temporary,
        "scan",
        Duration::from_secs(930),
        8 * 1024 * 1024,
    )?;
    let code = result
        .status
        .code()
        .ok_or("The scanner exited without a recognized status.")?;
    if !result.empty() {
        return Err("Gitleaks reported scanner errors; no clean result was recorded.");
    }
    verify_child_status(&status_path)?;
    if fs::metadata(&report_path)
        .map_err(|_| "Gitleaks did not produce a complete report.")?
        .len()
        > 64 * 1024 * 1024
    {
        return Err("The scanner report exceeds its bounded input size.");
    }
    let findings = report::validate_report(
        &fs::read(report_path).map_err(|_| "The complete scanner report could not be read.")?,
        code,
    )?;
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        report::write(Path::new(&path), &findings, scope)?;
    }
    if findings.is_empty() {
        log(
            Level::Info,
            "The scan completed successfully; no matching secrets were detected.",
        );
        Ok(0)
    } else {
        log(
            Level::Warning,
            "Potential secrets were detected; review only the redacted locations in the job summary.",
        );
        Ok(1)
    }
}
fn main() -> ExitCode {
    let is_wrapper = std::env::args_os()
        .next()
        .and_then(|p| {
            Path::new(&p)
                .file_name()
                .map(|n| n == "git" || n == "git.exe")
        })
        .unwrap_or(false);
    let result = if is_wrapper { wrapper() } else { scan() };
    match result {
        Ok(code) => ExitCode::from(if code == 0 {
            0
        } else if code > 0 && code < 256 {
            code as u8
        } else {
            1
        }),
        Err(message) => {
            if is_wrapper {
                eprintln!("ERROR: {message}");
            } else {
                log(Level::Error, message);
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_failed_git_observation_never_reports_success() {
        let dir = PrivateDir::new(&std::env::temp_dir()).unwrap();
        let status = dir.0.join("git-status");
        assert!(verify_child_status(&status).is_err());
        for value in ["", "1", "-1", "0\n", "0\n1"] {
            fs::write(&status, value).unwrap();
            assert!(verify_child_status(&status).is_err());
        }
        fs::write(&status, "0").unwrap();
        assert!(verify_child_status(&status).is_ok());
    }

    #[test]
    fn only_full_hexadecimal_revisions_reach_git_arguments() {
        assert!(valid_revision("0123456789abcdef0123456789abcdef01234567"));
        for revision in [
            "",
            "HEAD",
            "--all",
            "abc..def",
            "0123456789abcdef0123456789abcdef0123456g",
        ] {
            assert!(!valid_revision(revision));
        }
    }
}
