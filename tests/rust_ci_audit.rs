#![allow(clippy::unwrap_used)]
#[allow(dead_code)]
#[path = "../src/bin/matrix-evidence.rs"]
mod gate;
#[allow(dead_code)]
#[path = "../src/bin/migration-audit.rs"]
mod migration;
#[allow(dead_code)]
#[path = "../tools/secret-scan/src/main.rs"]
mod scanner;
#[path = "support_ci/scanner_limits.rs"]
mod scanner_limits;
#[path = "support_ci/mod.rs"]
mod support;
#[allow(dead_code)]
mod support_live;
#[allow(dead_code)]
mod support_live_harness;
#[test]
fn complete_identical_matrix_is_accepted() {
    support::stream_orderings();
    let one: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::from([("legacy-a".into(), "native-a".into())]);
    let two: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::from([("legacy-b".into(), "native-b".into())]);
    let all: std::collections::BTreeMap<String, String> = one
        .iter()
        .chain(&two)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert!(migration::check_partitions(&all, &[one.clone(), two.clone()]).is_ok());
    assert!(migration::check_partitions(&all, std::slice::from_ref(&one)).is_err());
    assert!(migration::check_partitions(&all, &[one.clone(), two.clone(), one.clone()]).is_err());
    let mut changed = two.clone();
    changed.insert("legacy-b".into(), "wrong-target".into());
    assert!(migration::check_partitions(&all, &[one.clone(), changed]).is_err());
    let mut changed = two;
    changed.remove("legacy-b");
    changed.insert("different-legacy-id".into(), "native-b".into());
    assert!(migration::check_partitions(&all, &[one, changed]).is_err());
    support::coverage::valid();
    let report = scanner::report::validate_report(b"[]", 0).unwrap();
    assert!(report.is_empty());
    assert!(scanner::valid_revision(&"a".repeat(40)));
    for invalid in ["--all", "../HEAD", "bad", &"g".repeat(40)] {
        assert!(!scanner::valid_revision(invalid));
    }
    let directory = support::folder();
    let policy = support::policy();
    support::matrix(directory.path(), &policy, true);
    let summary = gate::check_matrix(directory.path(), &policy, gate::LEGS.len()).unwrap();
    assert!(summary.contains("1851"));
    assert!(gate::Policy::from_manifest("[]").is_err());
    assert!(
        gate::Policy::from_manifest(&serde_json::to_string(&vec!["duplicate"; 1851]).unwrap())
            .is_err()
    );
    assert_eq!(
        gate::strip_ansi("\x1b[1m\x1b[92m Running\x1b[0m tests/rust_core.rs (target/test)"),
        " Running tests/rust_core.rs (target/test)"
    );
    let unicode = "Running tests/فضا.rs\ntest 日本語 ... ok\n";
    assert_eq!(gate::strip_ansi(unicode), unicode);
}
#[test]
fn rejects_missing() {
    support::stream_rejections("missing");
    support::false_green("missing");
    for leg in gate::LEGS.into_iter().filter(|leg| leg.contains("aarch64")) {
        let directory = support::folder();
        let policy = support::policy();
        support::matrix(directory.path(), &policy, false);
        std::fs::remove_dir_all(directory.path().join(leg)).unwrap();
        let error = gate::check_matrix(directory.path(), &policy, gate::LEGS.len()).unwrap_err();
        assert!(
            error.contains(leg),
            "missing ARM evidence must fail: {error}"
        );
    }
}
#[test]
fn rejects_extra() {
    support::false_green("extra");
}
#[test]
fn rejects_different() {
    support::stream_rejections("counts");
    support::false_green("different");
}
#[test]
fn rejects_skipped() {
    support::false_green("skipped");
}
#[test]
fn rejects_failed() {
    scanner_limits::completed_child_remains_bounded();
    support::coverage::invalid();
    support::false_green("failed");
    assert!(scanner::report::validate_report(b"[]", 2).is_err());
    assert!(scanner::report::validate_report(b"[]", 1).is_err());
    assert!(scanner::report::validate_report(b"[{}]", 0).is_err());
    let finding = serde_json::json!({"RuleID":"rule", "File":"<script>|line\nnext", "StartLine":1,"Commit":"abcdef","Secret":"PUBLIC-BURNED-CANARY-NOT-FOR-SUMMARY","Match":"PUBLIC-BURNED-CANARY-NOT-FOR-SUMMARY","Message":"PUBLIC-BURNED-CANARY-NOT-FOR-SUMMARY"});
    let accepted =
        scanner::report::validate_report(&serde_json::to_vec(&vec![finding]).unwrap(), 2).unwrap();
    let summary = scanner::report::summary(&accepted, "public fixture");
    assert!(!summary.contains("PUBLIC-BURNED-CANARY-NOT-FOR-SUMMARY"));
    assert!(summary.contains("&lt;script&gt;&#124;line next"));
    let directory = tempfile::tempdir().unwrap();
    let status = directory.path().join("status");
    assert!(scanner::verify_child_status(&status).is_err());
    std::fs::write(&status, b"128").unwrap();
    assert!(scanner::verify_child_status(&status).is_err());
    std::fs::write(&status, b"0").unwrap();
    scanner::verify_child_status(&status).unwrap();
}
#[test]
fn rejects_empty() {
    support::false_green("empty");
}
#[test]
fn rejects_duplicate() {
    support::stream_rejections("duplicate");
    support::false_green("duplicate");
}
#[test]
fn summary_counts_actual_live_skips() {
    let d = support::folder();
    let p = support::policy();
    support::matrix(&d.path().join("records"), &p, false);
    let output = support::cli(d.path(), &p, gate::LEGS.len());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains(&format!(
        "The {} live Telegram cases",
        gate::LIVE_CASES.len()
    )));
}
#[test]
fn live_case_list_matches_ignored_interop_tier() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(support_live_harness::Pump::verify_cancelled_pause_aborts_task());
    let source = syn::parse_file(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rust_live_interop.rs"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut found = std::collections::BTreeSet::new();
    for item in source.items {
        if let syn::Item::Fn(function) = item {
            let test = function
                .attrs
                .iter()
                .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"));
            if test {
                assert!(
                    function.attrs.iter().any(|a| a.path().is_ident("ignore")),
                    "live tests must require explicit operator invocation"
                );
                found.insert(format!(
                    "tests/rust_live_interop.rs::{}",
                    function.sig.ident
                ));
            }
        }
    }
    assert_eq!(
        found,
        gate::LIVE_CASES.into_iter().map(str::to_owned).collect()
    );
}
#[test]
fn platform_skip_is_accepted_on_declared_leg() {
    let p = support::policy();
    assert_eq!(p.platform.len(), 2);
    for leg in gate::WINDOWS_LEGS {
        let record = support::record(&p, leg, false, false);
        gate::parse_record(&record, leg, &p).unwrap();
        assert_eq!(p.platform[leg].len(), 3);
    }
}
#[test]
fn platform_skip_is_rejected_on_other_leg() {
    let p = support::policy();
    let record = support::record(&p, gate::WINDOWS_LEGS[0], false, false);
    for leg in gate::LEGS
        .into_iter()
        .filter(|leg| !gate::WINDOWS_LEGS.contains(leg))
    {
        let error = gate::parse_record(&record, leg, &p).unwrap_err();
        assert!(error.contains(gate::WINDOWS_CASES[0]));
    }
}
#[test]
fn rejection_names_the_changed_identity() {
    let d = support::folder();
    let p = support::policy();
    let root = d.path().join("records");
    support::matrix(&root, &p, false);
    support::mutate(&root, gate::LEGS[1], "scenario_0000", "only_on_leg_b");
    let output = support::cli(d.path(), &p, gate::LEGS.len());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("tests/rust_ci_fixture.rs::only_on_leg_b")
    );
}
#[test]
fn exact_declared_leg_count_succeeds() {
    let d = support::folder();
    let p = support::policy();
    support::matrix(&d.path().join("records"), &p, false);
    assert!(
        support::cli(d.path(), &p, gate::LEGS.len())
            .status
            .success()
    );
}
#[test]
fn incorrect_declared_leg_count_is_rejected() {
    let d = support::folder();
    let p = support::policy();
    support::matrix(&d.path().join("records"), &p, false);
    assert_eq!(
        support::cli(d.path(), &p, gate::LEGS.len() - 1)
            .status
            .code(),
        Some(1)
    );
}
fn optional(skip: bool) {
    let mut p = support::policy();
    p.optional
        .insert("tests/rust_ci_fixture.rs::optional_capture".into());
    gate::parse_record(
        &support::record(&p, gate::LEGS[0], true, skip),
        gate::LEGS[0],
        &p,
    )
    .unwrap();
}
#[test]
fn optional_fixture_can_be_skipped() {
    optional(true);
}
#[test]
fn optional_fixture_can_run() {
    optional(false);
}
