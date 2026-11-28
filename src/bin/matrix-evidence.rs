//! Fail closed on incomplete Rust execution evidence, including declared skips.
#![forbid(unsafe_code)]

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;
use common::{Level, log};
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
pub const SUPPLEMENTAL_CASES: [&str; 9] = [
    "unittests src/lib.rs::aes_ige::tests::decryption_rejects_partial_blocks - should panic",
    "unittests src/lib.rs::aes_ige::tests::encryption_rejects_partial_blocks - should panic",
    "unittests src/lib.rs::aes_ige::tests::streaming_matches_reference",
    "unittests src/lib.rs::aes_ige::tests::unaligned_buffers_match_reference",
    "unittests src/lib.rs::chat::validation::signed_secret_chat_id_validates_only_when_nonzero",
    "unittests src/lib.rs::ingress::strong_tests::signed_secret_chat_id_for_strong_replay_must_be_nonzero",
    "unittests src/lib.rs::ingress::strong_tests::signed_secret_chat_id_for_unknown_discard_must_be_nonzero",
    "unittests src/lib.rs::storage::lifecycle_tests::signed_secret_chat_id_is_retained_in_malformed_record_errors",
    "unittests src/lib.rs::storage::lifecycle_tests::signed_secret_chat_id_survives_file_reopen",
];
pub const LEGS: [&str; 7] = [
    "rust-ubuntu-24.04-x86_64-1.95.0-debug",
    "rust-ubuntu-24.04-x86_64-1.100.0-debug",
    "rust-ubuntu-24.04-x86_64-1.100.0-release",
    "rust-ubuntu-24.04-x86_64-1.100.0-portable-aes",
    "rust-ubuntu-24.04-arm-aarch64-1.100.0-release",
    "rust-windows-2025-x86_64-1.100.0-release",
    "rust-windows-11-arm-aarch64-1.100.0-release",
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
    excluded: BTreeMap<String, BTreeSet<String>>,
    supplemental: &'static [&'static str],
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentManifest {
    expected: Vec<String>,
    #[serde(default)]
    platform_excluded: BTreeMap<String, BTreeSet<String>>,
}

impl Policy {
    fn current(text: &str) -> Result<Self, String> {
        let manifest: CurrentManifest =
            serde_json::from_str(text).map_err(|_| "Invalid current Rust matrix manifest")?;
        let mut live = [
            "tests/live_interop_harness.rs::same_library_pair",
            "tests/live_interop_harness.rs::official_client_interop",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        let expected: BTreeSet<_> = manifest.expected.iter().cloned().collect();
        live.extend(
            LIVE_CASES
                .iter()
                .filter(|id| expected.contains(**id))
                .map(|id| (*id).to_owned()),
        );
        let platform_ignored: BTreeSet<_> = WINDOWS_CASES
            .iter()
            .filter(|id| expected.contains(**id))
            .map(|id| (*id).to_owned())
            .collect();
        if expected.len() != manifest.expected.len()
            || !live.is_subset(&expected)
            || expected.len() == live.len()
            || manifest.platform_excluded.iter().any(|(leg, cases)| {
                !LEGS.contains(&leg.as_str())
                    || !cases.is_subset(&expected)
                    || !cases.is_disjoint(&live)
                    || !cases.is_disjoint(&platform_ignored)
            })
            || expected.iter().any(|identity| {
                !live.contains(identity)
                    && LEGS.iter().all(|leg| {
                        manifest
                            .platform_excluded
                            .get(*leg)
                            .is_some_and(|cases| cases.contains(identity))
                    })
            })
        {
            return Err(
                "Current Rust matrix manifest has invalid testcase or platform declarations".into(),
            );
        }
        Ok(Self {
            expected,
            live,
            platform: WINDOWS_LEGS
                .map(|leg| (leg.to_owned(), platform_ignored.clone()))
                .into_iter()
                .collect(),
            optional: BTreeSet::new(),
            excluded: manifest.platform_excluded,
            supplemental: &[],
        })
    }

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
            excluded: BTreeMap::new(),
            supplemental: &SUPPLEMENTAL_CASES,
        })
    }
}
#[derive(Default)]
struct Suite {
    declared: usize,
    rows: Vec<(String, bool)>,
    passed: usize,
    ignored: usize,
    open: bool,
}
fn count(summary: &str, word: &str) -> Result<usize, String> {
    let tokens = summary.split_whitespace();
    tokens
        .clone()
        .zip(tokens.skip(1))
        .find(|(_, next)| *next == word)
        .and_then(|(value, _)| value.parse().ok())
        .ok_or_else(|| format!("malformed libtest {word} summary"))
}
pub fn parse_record(text: &str, leg: &str, policy: &Policy) -> Result<BTreeSet<String>, String> {
    let mut expected_all = policy.expected.clone();
    if let Some(excluded) = policy.excluded.get(leg) {
        expected_all.retain(|id| !excluded.contains(id));
    }
    for &id in policy.supplemental {
        if !expected_all.insert(id.to_owned()) {
            return Err(format!(
                "{leg}: supplemental testcase identity overlap: {id}"
            ));
        }
    }
    let text = strip_ansi(text);
    if text.contains("test result: FAILED") || text.contains("error: test failed") {
        return Err(format!("{leg}: failing test run"));
    }
    let mut suite = Suite::default();
    let mut labels = Vec::new();
    let mut suites = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(run) = line.strip_prefix("Running ") {
            let label = run
                .split(" (")
                .next()
                .ok_or("missing Cargo suite label")?
                .replace('\\', "/");
            if !(label.starts_with("tests/") || label.starts_with("unittests ")) {
                return Err(format!("{leg}: unexpected Cargo test suite {label}"));
            }
            labels.push(label);
            continue;
        }
        if policy.supplemental.is_empty() && line.starts_with("Doc-tests ") {
            labels.push(line.to_owned());
            continue;
        }
        if let Some(declared) = line.strip_prefix("running ").and_then(|value| {
            value
                .strip_suffix(" tests")
                .or_else(|| value.strip_suffix(" test"))
        }) {
            if suite.open {
                return Err(format!("{leg}: a test block has no result summary"));
            }
            suite = Suite {
                declared: declared
                    .parse()
                    .map_err(|_| format!("{leg}: invalid declared test count"))?,
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
                || passed.checked_add(skip) != Some(suite.declared)
            {
                return Err(format!(
                    "{leg}: summary and declared counts disagree with actual executed rows"
                ));
            }
            suite.open = false;
            suites.push(std::mem::take(&mut suite));
            continue;
        }
        if let Some(test) = line.strip_prefix("test ")
            && let Some((name, status)) = test.split_once(" ... ")
        {
            if !suite.open {
                return Err(format!("{leg}: test row outside a running suite"));
            }
            if status == "ok" {
                suite.passed += 1;
                suite.rows.push((name.to_owned(), false));
            } else if status == "ignored" || status.starts_with("ignored,") {
                suite.ignored += 1;
                suite.rows.push((name.to_owned(), true));
            } else {
                return Err(format!(
                    "{leg}: failed or unknown status for {name}: {status}"
                ));
            }
        }
    }
    if suite.open || suites.is_empty() || labels.len() != suites.len() {
        return Err(format!("{leg}: incomplete or empty executed test record"));
    }
    // Cargo writes headers to stderr; libtest writes blocks to stdout. Native
    // Windows capture can interleave these streams, but each stream preserves
    // Cargo's sequential suite order. Pair only after checking both complete
    // inventories, never by guessing which case names belong to a suite.
    let mut cases = BTreeSet::new();
    let mut ignored = BTreeSet::new();
    for (label, suite) in labels.into_iter().zip(suites) {
        for (name, skipped) in suite.rows {
            let id = format!("{label}::{name}");
            if !cases.insert(id.clone()) {
                return Err(format!("{leg}: duplicate testcase identity {id}"));
            }
            if skipped {
                ignored.insert(id);
            }
        }
    }
    if cases.is_empty() {
        return Err(format!("{leg}: incomplete or empty executed test record"));
    }
    for &id in policy.supplemental {
        if ignored.contains(id) {
            return Err(format!("{leg}: cannot skip supplemental testcase {id}"));
        }
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
    if cases != expected_all {
        let extra = cases
            .difference(&expected_all)
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        let missing = expected_all
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
    for leg in LEGS {
        let text = fs::read_to_string(root.join(leg).join("rust-tests.txt"))
            .map_err(|_| format!("{leg}: executed test log is missing or not UTF-8"))?;
        parse_record(&text, leg, policy)?;
    }
    Ok(format!(
        "Verified all {} Rust matrix legs against the declared inventory of {} test cases. The {} live Telegram cases were not run.",
        LEGS.len(),
        policy.expected.len() + policy.supplemental.len(),
        policy.live.len(),
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
const USAGE: &str = "Usage: matrix-evidence RECORDS [--manifest LEGACY_JSON | --current-manifest JSON] [--expect-legs N]\n       matrix-evidence --coverage LLVM_JSON\n\nThe default is the bundled current executable inventory. Current JSON lists unique test identities in expected and, optionally, a platform_excluded object mapping leg names to cfg-absent identities. Declared live tests and the three Windows filesystem contract ignores remain collected and are verified separately. --manifest selects the historical migration inventory.";

pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<String, String> {
    let mut directory = PathBuf::from("records");
    let mut have_directory = false;
    let mut manifest = None;
    let mut count = LEGS.len();
    let mut supplied = false;
    let mut current = true;
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--current-manifest" if manifest.is_none() => {
                current = true;
                manifest = Some(PathBuf::from(
                    args.next().ok_or("--current-manifest requires a path")?,
                ));
            }
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
            "--manifest" if manifest.is_none() => {
                current = false;
                manifest = Some(PathBuf::from(
                    args.next().ok_or("--manifest requires a path")?,
                ));
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
                return Err(USAGE.into());
            }
        }
        supplied = true;
    }
    let manifest = if let Some(path) = manifest {
        fs::read_to_string(path).map_err(|_| "Rust case manifest could not be read")?
    } else {
        include_str!("../../migration/current-rust-cases.json").to_owned()
    };
    let policy = if current {
        Policy::current(&manifest)?
    } else {
        Policy::from_manifest(&manifest)?
    };
    check_matrix(&directory, &policy, count)
}
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let [flag] = args.as_slice() {
        match flag.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("matrix-evidence {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }
    match run(args) {
        Ok(summary) => {
            log(Level::Info, summary);
            ExitCode::SUCCESS
        }
        Err(error) => {
            log(
                Level::Error,
                format_args!("Rust matrix evidence rejected: {error}"),
            );
            ExitCode::FAILURE
        }
    }
}
