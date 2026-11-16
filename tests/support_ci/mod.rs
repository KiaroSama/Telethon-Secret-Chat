#![allow(clippy::unwrap_used)]
pub mod coverage;
use super::gate;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
pub fn policy() -> gate::Policy {
    let mut names = gate::LIVE_CASES
        .iter()
        .chain(gate::WINDOWS_CASES.iter())
        .map(|s| (*s).to_owned())
        .collect::<BTreeSet<_>>();
    names.insert("tests/فضا.rs::日本語".into());
    names.insert("tests/rust_ci_fixture.rs::optional_capture".into());
    let mut i = 0;
    while names.len() < gate::EXPECTED_CASES {
        names.insert(format!("tests/rust_ci_fixture.rs::scenario_{i:04}"));
        i += 1;
    }
    gate::Policy::from_manifest(&serde_json::to_string(&names).unwrap()).unwrap()
}
pub fn record(policy: &gate::Policy, leg: &str, color: bool, optional_skip: bool) -> String {
    let mut groups: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
    for id in &policy.expected {
        let (suite, name) = id.split_once("::").unwrap();
        let ignored = policy.live.contains(id)
            || policy.platform.get(leg).is_some_and(|p| p.contains(id))
            || (optional_skip && policy.optional.contains(id));
        groups.entry(suite).or_default().push((name, ignored));
    }
    let mut out = String::new();
    for (suite, rows) in groups {
        out.push_str(&format!(
            "{}Running{} {suite} (target/test)\n",
            if color { "\x1b[1m\x1b[92m " } else { "" },
            if color { "\x1b[0m" } else { "" }
        ));
        let mut ignored = 0;
        for (name, skip) in &rows {
            out.push_str(&format!(
                "test {name} ... {}\n",
                if *skip {
                    ignored += 1;
                    "ignored, requires explicit operator or platform"
                } else {
                    "ok"
                }
            ));
        }
        out.push_str(&format!("test result: ok. {} passed; 0 failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n",rows.len()-ignored));
    }
    out
}
pub fn matrix(root: &Path, policy: &gate::Policy, color: bool) {
    for leg in gate::LEGS {
        let folder = root.join(leg);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("rust-tests.txt"),
            record(policy, leg, color, false),
        )
        .unwrap();
    }
}
pub fn cli(root: &Path, policy: &gate::Policy, count: usize) -> std::process::Output {
    let manifest = root.join("manifest.json");
    fs::write(&manifest, serde_json::to_vec(&policy.expected).unwrap()).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_matrix-evidence"))
        .arg(root.join("records"))
        .arg("--manifest")
        .arg(&manifest)
        .arg("--expect-legs")
        .arg(count.to_string())
        .output()
        .unwrap()
}
pub fn folder() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("Rust CI Space Path ")
        .tempdir()
        .unwrap()
}
pub fn mutate(root: &Path, leg: &str, from: &str, to: &str) {
    let path = root.join(leg).join("rust-tests.txt");
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains(from), "mutation target absent");
    fs::write(path, source.replacen(from, to, 1)).unwrap();
}
pub fn false_green(scenario: &str) {
    let directory = folder();
    let policy = policy();
    matrix(directory.path(), &policy, false);
    let leg = gate::LEGS[1];
    match scenario{
"missing"=>fs::remove_dir_all(directory.path().join(leg)).unwrap(),
"extra"=>fs::create_dir(directory.path().join("unexpected")).unwrap(),
"different"=>mutate(directory.path(),leg,"scenario_0000","different_only_on_this_leg"),
"skipped"=>mutate(directory.path(),leg,"test scenario_0000 ... ok","test scenario_0000 ... ignored, unexpected"),
"failed"=>mutate(directory.path(),leg,"test scenario_0000 ... ok","test scenario_0000 ... FAILED"),
"empty"=>fs::write(directory.path().join(leg).join("rust-tests.txt"),"Running tests/empty.rs (target/test)\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n").unwrap(),
"duplicate"=>mutate(directory.path(),leg,"test scenario_0000 ... ok","test scenario_0000 ... ok\ntest scenario_0000 ... ok"),
_=>panic!("unknown matrix scenario")};
    assert!(gate::check_matrix(directory.path(), &policy, gate::LEGS.len()).is_err());
}
