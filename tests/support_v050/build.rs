#![forbid(unsafe_code)]

#[allow(dead_code)]
#[path = "../../src/bin/matrix-evidence.rs"]
mod gate;

use std::collections::BTreeMap;

fn current_fixture(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let live = [
        "tests/live_interop_harness.rs::same_library_pair",
        "tests/live_interop_harness.rs::official_client_interop",
    ];
    let expected = gate::WINDOWS_CASES
        .into_iter()
        .chain(live)
        .chain(["tests/current_fixture.rs::ordinary_case"])
        .collect::<Vec<_>>();
    let manifest = root.join("current.json");
    std::fs::write(
        &manifest,
        serde_json::to_vec(&serde_json::json!({"expected": expected})).unwrap(),
    )
    .unwrap();
    let records = root.join("records");
    for leg in gate::LEGS {
        let mut suites: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
        for id in &expected {
            let (suite, case) = id.split_once("::").unwrap();
            let ignored = live.contains(id)
                || (gate::WINDOWS_LEGS.contains(&leg) && gate::WINDOWS_CASES.contains(id));
            suites.entry(suite).or_default().push((case, ignored));
        }
        let mut text = String::new();
        for (suite, cases) in suites {
            text.push_str(&format!(
                "Running {suite} (target/fixture)\nrunning {} tests\n",
                cases.len()
            ));
            let ignored = cases.iter().filter(|(_, ignored)| *ignored).count();
            for (case, skipped) in &cases {
                text.push_str(&format!(
                    "test {case} ... {}\n",
                    if *skipped {
                        "ignored, declared platform or live requirement"
                    } else {
                        "ok"
                    }
                ));
            }
            text.push_str(&format!("test result: ok. {} passed; 0 failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s\n", cases.len() - ignored));
        }
        let folder = records.join(leg);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("rust-tests.txt"), text).unwrap();
    }
    (records, manifest)
}

#[test]
fn current_matrix_accepts_collected_windows_contract_ignores() {
    let folder = tempfile::tempdir().unwrap();
    let (records, manifest) = current_fixture(folder.path());
    let result = gate::run([
        records.to_string_lossy().into_owned(),
        "--current-manifest".into(),
        manifest.to_string_lossy().into_owned(),
    ]);
    assert!(
        result.is_ok(),
        "legitimate Windows contract ignores must remain verifiable: {result:?}"
    );
}

#[test]
fn every_mapped_native_integration_suite_remains_executable_source() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: Vec<String> =
        serde_json::from_str(include_str!("../../migration/rust-cases.json")).unwrap();
    let missing = cases
        .iter()
        .filter_map(|identity| identity.split_once("::"))
        .map(|(suite, _)| suite)
        .filter(|suite| suite.starts_with("tests/") && !root.join(suite).is_file())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        missing.is_empty(),
        "mapped native scenarios have no executable suite: {missing:?}"
    );
}
