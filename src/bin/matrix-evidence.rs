//! Fail closed on incomplete Rust execution evidence, including declared skips.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};
#[path = "matrix-evidence/coverage.rs"]
mod coverage;
pub use coverage::check_coverage;
pub const EXPECTED_CASES: usize = 1851;
pub const LEGS: [&str; 9] = [
    "rust-ubuntu-24.04-x86_64-1.95.0-debug",
    "rust-ubuntu-24.04-x86_64-1.100.0-debug",
    "rust-ubuntu-24.04-x86_64-1.100.0-release",
    "rust-ubuntu-24.04-x86_64-1.100.0-portable-aes",
    "rust-ubuntu-24.04-arm-aarch64-1.100.0-release",
    "rust-windows-2025-x86_64-1.100.0-release",
    "rust-windows-11-arm-aarch64-1.100.0-release",
    "rust-macos-15-intel-x86_64-1.100.0-release",
    "rust-macos-15-aarch64-1.100.0-release",
];
pub const LIVE_CASES: [&str; 10] = [
    "tests/rust_live_interop.rs::both_ends_agree_on_key_fingerprint",
    "tests/rust_live_interop.rs::message_crosses_both_directions",
    "tests/rust_live_interop.rs::closing_here_refuses_later_send",
    "tests/rust_live_interop.rs::media_opened_by_official_client",
    "tests/rust_live_interop.rs::incoming_file_decrypts_and_is_written",
    "tests/rust_live_interop.rs::official_client_accepts_rekey",
    "tests/rust_live_interop.rs::pair_text_rekey_files_and_restart",
    "tests/rust_live_interop.rs::peer_close_arrives_on_update_sync",
    "tests/rust_live_interop.rs::sending_after_peer_close_ends_local_chat",
    "tests/rust_live_interop.rs::delete_both_sides_removes_peer_chat",
];
pub const WINDOWS_LEGS: [&str; 2] = [
    "rust-windows-2025-x86_64-1.100.0-release",
    "rust-windows-11-arm-aarch64-1.100.0-release",
];
pub const WINDOWS_CASES: [&str; 3] = [
    "unittests src/lib.rs::storage::contract_tests::file_backend_is_owner_only",
    "unittests src/lib.rs::storage::guard_tests::symlinked_store_refuses_without_touching_target",
    "unittests src/lib.rs::storage::guard_tests::ordinary_directory_sync_failure_warns_after_whole_replace",
];
#[derive(Clone)]
pub struct Policy {
    pub expected: BTreeSet<String>,
    pub live: BTreeSet<String>,
    pub platform: BTreeMap<String, BTreeSet<String>>,
    pub optional: BTreeSet<String>,
}
impl Policy {
    pub fn from_manifest(text: &str) -> Result<Self, String> {
        let names: Vec<String> = serde_json::from_str(text)
            .map_err(|_| "Rust case manifest is not a JSON string array".to_owned())?;
        let expected: BTreeSet<_> = names.iter().cloned().collect();
        if names.len() != EXPECTED_CASES || expected.len() != EXPECTED_CASES {
            return Err(format!(
                "Rust case manifest must contain exactly {EXPECTED_CASES} unique identities; got {} rows and {} identities",
                names.len(),
                expected.len()
            ));
        }
        let live = LIVE_CASES
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>();
        let windows = WINDOWS_CASES
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>();
        for id in live.iter().chain(&windows) {
            if !expected.contains(id) {
                return Err(format!("declared skip absent from Rust manifest: {id}"));
            }
        }
        Ok(Self {
            expected,
            live,
            platform: WINDOWS_LEGS
                .map(|leg| (leg.to_owned(), windows.clone()))
                .into_iter()
                .collect(),
            optional: BTreeSet::new(),
        })
    }
}
#[derive(Default)]
struct Suite {
    label: String,
    passed: usize,
    ignored: usize,
    open: bool,
}
fn count(summary: &str, word: &str) -> Result<usize, String> {
    let tokens = summary.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .find_map(|w| (w[1] == word).then(|| w[0].parse().ok()).flatten())
        .ok_or_else(|| format!("malformed libtest {word} summary"))
}
pub fn parse_record(text: &str, leg: &str, policy: &Policy) -> Result<BTreeSet<String>, String> {
    let text = strip_ansi(text);
    if text.contains("test result: FAILED") || text.contains("error: test failed") {
        return Err(format!("{leg}: failing test run"));
    }
    let mut suite = Suite::default();
    let mut cases = BTreeSet::new();
    let mut ignored = BTreeSet::new();
    let mut summaries = 0;
    for line in text.lines().map(str::trim) {
        if let Some(run) = line.strip_prefix("Running ") {
            if suite.open {
                return Err(format!("{leg}: no result summary for {}", suite.label));
            }
            let label = run
                .split(" (")
                .next()
                .ok_or("missing Cargo suite label")?
                .replace('\\', "/");
            if !(label.starts_with("tests/") || label.starts_with("unittests ")) {
                return Err(format!("{leg}: unexpected Cargo test suite {label}"));
            }
            suite = Suite {
                label,
                open: true,
                ..Suite::default()
            };
            continue;
        }
        if line.starts_with("test result: ok.") {
            if !suite.open {
                return Err(format!("{leg}: summary without a test suite"));
            }
            let passed = count(line, "passed;")?;
            let failed = count(line, "failed;")?;
            let skip = count(line, "ignored;")?;
            let measured = count(line, "measured;")?;
            let filtered = count(line, "filtered")?;
            if passed != suite.passed
                || skip != suite.ignored
                || failed != 0
                || measured != 0
                || filtered != 0
            {
                return Err(format!(
                    "{leg}: summary counts disagree with actual executed rows for {}",
                    suite.label
                ));
            }
            suite.open = false;
            summaries += 1;
            continue;
        }
        if let Some(test) = line.strip_prefix("test ")
            && let Some((name, status)) = test.split_once(" ... ")
        {
            if !suite.open {
                return Err(format!("{leg}: test row outside a running suite"));
            }
            let id = format!("{}::{name}", suite.label);
            if !cases.insert(id.clone()) {
                return Err(format!("{leg}: duplicate testcase identity {id}"));
            }
            if status == "ok" {
                suite.passed += 1;
            } else if status == "ignored" || status.starts_with("ignored,") {
                suite.ignored += 1;
                ignored.insert(id);
            } else {
                return Err(format!(
                    "{leg}: failed or unknown status for {id}: {status}"
                ));
            }
        }
    }
    if suite.open || summaries == 0 || cases.is_empty() {
        return Err(format!("{leg}: incomplete or empty executed test record"));
    }
    let mut required = policy.live.clone();
    if let Some(platform) = policy.platform.get(leg) {
        required.extend(platform.iter().cloned());
    }
    let allowed = required
        .union(&policy.optional)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !required.is_subset(&ignored) || !ignored.is_subset(&allowed) {
        let unexpected = ignored
            .difference(&allowed)
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        let missing = required
            .difference(&ignored)
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "{leg}: skip declarations differ; unexpected {unexpected:?}; missing {missing:?}"
        ));
    }
    if cases.len() == ignored.len() {
        return Err(format!("{leg}: no offline tests executed"));
    }
    if cases != policy.expected {
        let extra = cases
            .difference(&policy.expected)
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        let missing = policy
            .expected
            .difference(&cases)
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "{leg}: testcase identities differ; extra {extra:?}; missing {missing:?}"
        ));
    }
    Ok(cases)
}
pub fn check_matrix(root: &Path, policy: &Policy, expect_legs: usize) -> Result<String, String> {
    if expect_legs != LEGS.len() {
        return Err(format!(
            "expected {expect_legs} matrix legs, but the declared Rust matrix contains {}",
            LEGS.len()
        ));
    }
    if policy.expected.len() != EXPECTED_CASES {
        return Err("matrix policy does not contain the strict 1851-case manifest".into());
    }
    let actual = fs::read_dir(root)
        .map_err(|_| "matrix evidence directory could not be read")?
        .map(|entry| {
            entry
                .map_err(|_| "matrix artifact could not be read".to_owned())
                .and_then(|entry| {
                    if !entry
                        .file_type()
                        .map_err(|_| "matrix artifact metadata unavailable")?
                        .is_dir()
                    {
                        return Err("matrix artifacts must be directories".into());
                    }
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "matrix artifact name is not UTF-8".into())
                })
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    let expected = LEGS
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "missing or unexpected matrix artifacts; expected {expected:?}; found {actual:?}"
        ));
    }
    let mut baseline = None;
    for leg in LEGS {
        let text = fs::read_to_string(root.join(leg).join("rust-tests.txt"))
            .map_err(|_| format!("{leg}: executed test log is missing or not UTF-8"))?;
        let cases = parse_record(&text, leg, policy)?;
        if baseline.as_ref().is_some_and(|base| base != &cases) {
            return Err(format!(
                "{leg}: collected testcase identities differ from earlier legs"
            ));
        }
        baseline = Some(cases);
    }
    Ok(format!(
        "The {} live Telegram cases were intentionally NOT executed. All {} Rust matrix legs agree on {} testcase identities.",
        policy.live.len(),
        LEGS.len(),
        policy.expected.len()
    ))
}
pub fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for control in chars.by_ref() {
                if ('@'..='~').contains(&control) {
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}
pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<String, String> {
    let mut directory = PathBuf::from("records");
    let mut have_directory = false;
    let mut manifest = None;
    let mut count = LEGS.len();
    let mut supplied = false;
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--coverage" if !supplied => {
                let path = args.next().ok_or("--coverage requires a summary path")?;
                if args.next().is_some() {
                    return Err("--coverage must be used alone with its summary path".into());
                }
                let bytes = fs::read(path).map_err(|_| "Coverage summary could not be read")?;
                let root = std::env::current_dir()
                    .map_err(|_| "Coverage source checkout is unavailable")?;
                return check_coverage(&bytes, &root);
            }
            "--manifest" => {
                manifest = Some(PathBuf::from(
                    args.next().ok_or("--manifest requires a path")?,
                ))
            }
            "--expect-legs" => {
                count = args
                    .next()
                    .ok_or("--expect-legs requires a count")?
                    .parse()
                    .map_err(|_| "invalid expected leg count")?
            }
            _ if !arg.starts_with('-') && !have_directory => {
                directory = arg.into();
                have_directory = true;
            }
            _ => {
                return Err(
                    "usage: matrix-evidence RECORDS [--manifest PATH] [--expect-legs N]".into(),
                );
            }
        }
        supplied = true;
    }
    let manifest = if let Some(path) = manifest {
        fs::read_to_string(path).map_err(|_| "Rust case manifest could not be read")?
    } else {
        include_str!("../../migration/rust-cases.json").to_owned()
    };
    let policy = Policy::from_manifest(&manifest)?;
    check_matrix(&directory, &policy, count)
}
fn main() -> ExitCode {
    match run(std::env::args().skip(1)) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ERROR: Rust matrix evidence rejected: {error}");
            ExitCode::FAILURE
        }
    }
}
