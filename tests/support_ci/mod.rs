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
const AES_IGE_CASES: [&str; 4] = [
    "unittests src/lib.rs::aes_ige::tests::decryption_rejects_partial_blocks - should panic",
    "unittests src/lib.rs::aes_ige::tests::encryption_rejects_partial_blocks - should panic",
    "unittests src/lib.rs::aes_ige::tests::streaming_matches_reference",
    "unittests src/lib.rs::aes_ige::tests::unaligned_buffers_match_reference",
];
fn expected_ids(policy: &gate::Policy) -> impl Iterator<Item = &str> {
    policy
        .expected
        .iter()
        .map(String::as_str)
        .chain(AES_IGE_CASES)
}
pub fn record(policy: &gate::Policy, leg: &str, color: bool, optional_skip: bool) -> String {
    let mut groups: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
    for id in expected_ids(policy) {
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
        out.push_str(&format!(
            "running {} test{}\n",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
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
"empty"=>fs::write(directory.path().join(leg).join("rust-tests.txt"),"Running tests/empty.rs (target/test)\nrunning 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n").unwrap(),
"duplicate"=>mutate(directory.path(),leg,"test scenario_0000 ... ok","test scenario_0000 ... ok\ntest scenario_0000 ... ok"),
_=>panic!("unknown matrix scenario")};
    assert!(gate::check_matrix(directory.path(), &policy, gate::LEGS.len()).is_err());
}
fn stream_parts(policy: &gate::Policy, leg: &str) -> (Vec<String>, Vec<String>) {
    let record = record(policy, leg, false, false);
    let mut headers = Vec::new();
    let mut blocks = Vec::new();
    for section in record.split("\n\n").filter(|s| !s.is_empty()) {
        let (header, body) = section.split_once('\n').unwrap();
        headers.push(format!("{header}\n"));
        blocks.push(format!("{body}\n\n"));
    }
    assert_eq!(headers.len(), blocks.len());
    (headers, blocks)
}
pub fn stream_orderings() {
    let policy = policy();
    for leg in gate::LEGS {
        let (headers, blocks) = stream_parts(&policy, leg);
        for text in [
            format!("{}{}", headers.concat(), blocks.concat()),
            format!("{}{}", blocks.concat(), headers.concat()),
        ] {
            assert_eq!(
                gate::parse_record(&text, leg, &policy).unwrap(),
                expected_ids(&policy).map(str::to_owned).collect()
            );
        }
    }
}
pub fn stream_rejections(kind: &str) {
    let policy = policy();
    let leg = gate::WINDOWS_LEGS[0];
    let (headers, blocks) = stream_parts(&policy, leg);
    let valid = format!("{}{}", headers.concat(), blocks.concat());
    assert_eq!(
        gate::parse_record(&valid, leg, &policy).unwrap(),
        expected_ids(&policy).map(str::to_owned).collect()
    );
    let mut invalid = Vec::new();
    match kind {
        "missing" => {
            let row = blocks[0]
                .lines()
                .find(|line| line.starts_with("test "))
                .unwrap();
            invalid.push(valid.replacen(&format!("{row}\n"), "", 1));
            invalid.push(format!("{}{}", headers[1..].concat(), blocks.concat()));
            let summary = blocks[0]
                .lines()
                .find(|line| line.starts_with("test result:"))
                .unwrap();
            invalid.push(valid.replacen(&format!("{summary}\n"), "", 1));
            invalid.push(format!("{}{}", headers.concat(), blocks[1..].concat()));
        }
        "duplicate" => {
            invalid.push(format!(
                "{}{}{}",
                headers[0],
                headers.concat(),
                blocks.concat()
            ));
            invalid.push(format!(
                "{}{}{}",
                headers.concat(),
                blocks[0],
                blocks.concat()
            ));
            invalid.push(format!(
                "{}{}{}{}",
                headers[0],
                headers.concat(),
                blocks[0],
                blocks.concat()
            ));
        }
        "counts" => {
            let declared = blocks[0].lines().next().unwrap();
            let number: usize = declared.split_whitespace().nth(1).unwrap().parse().unwrap();
            invalid.push(valid.replacen(declared, &format!("running {} tests", number + 1), 1));
            invalid.push(valid.replacen(declared, "running malformed tests", 1));
            invalid.push(valid.replacen(&format!("{declared}\n"), "", 1));
        }
        _ => panic!("unknown stream rejection scenario"),
    }
    for text in invalid {
        assert!(
            gate::parse_record(&text, leg, &policy).is_err(),
            "invalid {kind} stream accepted"
        );
    }
}
