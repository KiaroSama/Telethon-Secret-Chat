use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn valid_junit_xml(extra_skip: bool) -> String {
    let live_skips = [
        (
            "tests.interop.test_live_roundtrip",
            "test_both_ends_agree_on_the_key_fingerprint",
        ),
        (
            "tests.interop.test_live_roundtrip",
            "test_a_message_crosses_in_both_directions",
        ),
        (
            "tests.interop.test_live_roundtrip",
            "test_closing_here_is_seen_as_closed_here",
        ),
        (
            "tests.interop.test_live_media",
            "test_every_media_kind_is_sent_and_opens_on_the_far_side",
        ),
        (
            "tests.interop.test_live_media",
            "test_a_file_from_the_far_side_decrypts_and_is_written",
        ),
        (
            "tests.interop.test_live_rekey",
            "test_a_rekey_started_here_is_accepted_by_the_official_client",
        ),
        (
            "tests.live_pair.test_pair_chat",
            "test_one_chat_through_text_rekey_files_and_a_restart",
        ),
        (
            "tests.live_pair.test_pair_close",
            "test_a_close_reaches_the_other_side_on_its_next_update_sync",
        ),
        (
            "tests.live_pair.test_pair_close",
            "test_sending_into_a_chat_the_peer_closed_closes_it_here",
        ),
        (
            "tests.live_pair.test_pair_delete",
            "test_a_delete_for_both_sides_removes_the_chat_on_the_other_side",
        ),
    ];

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<testsuite tests=\"12\" failures=\"0\" errors=\"0\">\n",
    );
    if extra_skip {
        xml.push_str(
            "  <testcase classname=\"tests.unit.test_framing\" name=\"test_valid_layer\"><skipped message=\"unexpected\" /></testcase>\n",
        );
    } else {
        xml.push_str(
            "  <testcase classname=\"tests.unit.test_framing\" name=\"test_valid_layer\" />\n",
        );
    }
    xml.push_str("  <testcase classname=\"tests.unit.test_crypto\" name=\"test_kdf\" />\n");
    for (cls, name) in live_skips {
        xml.push_str(&format!(
            "  <testcase classname=\"{cls}\" name=\"{name}\"><skipped message=\"live\" /></testcase>\n"
        ));
    }
    xml.push_str("</testsuite>\n");
    xml
}

#[test]
fn check_test_matrix_binary_accepts_valid_matrix_and_rejects_drift_without_python() {
    let bin = env!("CARGO_BIN_EXE_check-test-matrix");
    let dir = tempfile::tempdir().expect("tempdir");

    let valid_xml = valid_junit_xml(false);
    for leg in ["linux-x64", "windows-x64"] {
        let leg_dir = dir.path().join(leg);
        std::fs::create_dir_all(&leg_dir).unwrap();
        std::fs::write(leg_dir.join("junit.xml"), &valid_xml).unwrap();
    }

    let ok = Command::new(bin)
        .arg(dir.path())
        .args(["--expect-legs", "2", "linux-x64", "windows-x64"])
        .output()
        .expect("run check-test-matrix");
    assert!(
        ok.status.success(),
        "expected exit 0, got {:?}: stderr={}",
        ok.status.code(),
        String::from_utf8_lossy(&ok.stderr)
    );

    // Expecting 3 legs when only 2 are supplied must fail closed with exit 1.
    let bad_count = Command::new(bin)
        .arg(dir.path())
        .args(["--expect-legs", "3", "linux-x64", "windows-x64"])
        .output()
        .expect("run check-test-matrix mismatch");
    assert_eq!(bad_count.status.code(), Some(1));

    // Unexpected skip in one leg must fail closed with exit 1.
    std::fs::write(
        dir.path().join("windows-x64/junit.xml"),
        valid_junit_xml(true),
    )
    .unwrap();
    let bad_skip = Command::new(bin)
        .arg(dir.path())
        .args(["--expect-legs", "2", "linux-x64", "windows-x64"])
        .output()
        .expect("run check-test-matrix skip");
    assert_eq!(bad_skip.status.code(), Some(1));
}

#[test]
fn generate_schema_binary_generates_and_checks_tracked_tl_schema_without_python() {
    let bin = env!("CARGO_BIN_EXE_generate-schema");
    let root = repo_root();
    let schema = root.join("schema/end-to-end.tl");
    assert!(schema.exists(), "tracked schema/end-to-end.tl must exist");

    let dir = tempfile::tempdir().expect("tempdir");
    let generated_out = dir.path().join("secret_tl_generated.json");

    // Generate deterministic manifest/AST output from tracked end-to-end.tl.
    let gen_out = Command::new(bin)
        .args(["--schema"])
        .arg(&schema)
        .args(["--out"])
        .arg(&generated_out)
        .output()
        .expect("run generate-schema");
    assert!(
        gen_out.status.success(),
        "generate-schema failed: {}",
        String::from_utf8_lossy(&gen_out.stderr)
    );
    assert!(generated_out.exists());

    // --check against matching output exits 0.
    let check_ok = Command::new(bin)
        .args(["--check", "--schema"])
        .arg(&schema)
        .args(["--out"])
        .arg(&generated_out)
        .output()
        .expect("run generate-schema --check");
    assert!(check_ok.status.success());

    // Tampering with generated output causes --check to fail closed with exit 1.
    std::fs::write(&generated_out, b"drifted-content").unwrap();
    let check_drift = Command::new(bin)
        .args(["--check", "--schema"])
        .arg(&schema)
        .args(["--out"])
        .arg(&generated_out)
        .output()
        .expect("run generate-schema --check drifted");
    assert_eq!(check_drift.status.code(), Some(1));

    // Empty/invalid schema fails closed with exit 1.
    let empty_schema = dir.path().join("empty.tl");
    std::fs::write(&empty_schema, b"// empty schema\n").unwrap();
    let check_empty = Command::new(bin)
        .args(["--check", "--schema"])
        .arg(&empty_schema)
        .args(["--out"])
        .arg(&generated_out)
        .output()
        .expect("run generate-schema empty");
    assert_eq!(check_empty.status.code(), Some(1));
}

#[test]
fn maintenance_commands_support_help_and_reject_unknown_options() {
    for bin in [
        env!("CARGO_BIN_EXE_check-test-matrix"),
        env!("CARGO_BIN_EXE_generate-schema"),
    ] {
        let help = Command::new(bin).arg("--help").output().unwrap();
        assert!(help.status.success(), "{bin} --help must succeed");
        assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));
        let invalid = Command::new(bin).arg("--unknown-option").output().unwrap();
        assert_eq!(invalid.status.code(), Some(2));
    }
}

#[test]
fn check_test_matrix_rejects_an_incomplete_extra_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let expected = dir.path().join("linux-x64");
    std::fs::create_dir(&expected).unwrap();
    std::fs::write(expected.join("junit.xml"), valid_junit_xml(false)).unwrap();
    std::fs::create_dir(dir.path().join("unexpected-leg")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_check-test-matrix"))
        .arg(dir.path())
        .args(["--expect-legs", "1", "linux-x64"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "an incomplete artifact must not be silently discarded"
    );
}

#[test]
fn check_test_matrix_rejects_suite_failure_metadata_and_empty_case_names() {
    let dir = tempfile::tempdir().unwrap();
    let leg = dir.path().join("linux-x64");
    std::fs::create_dir(&leg).unwrap();
    for xml in [
        valid_junit_xml(false).replacen("failures=\"0\"", "failures=\"1\"", 1),
        valid_junit_xml(false).replacen("name=\"test_kdf\"", "name=\"\"", 1),
    ] {
        std::fs::write(leg.join("junit.xml"), xml).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_check-test-matrix"))
            .arg(dir.path())
            .args(["--expect-legs", "1", "linux-x64"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
    }
}

#[test]
fn generate_schema_preserves_preexisting_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let output_path = dir.path().join("manifest.json");
    let unrelated = dir.path().join("manifest.json.tmp");
    std::fs::write(&unrelated, b"unrelated caller-owned data").unwrap();
    std::fs::write(&output_path, b"previous manifest").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_generate-schema"))
        .arg("--schema")
        .arg(repo_root().join("schema/end-to-end.tl"))
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        std::fs::read(&unrelated).expect("caller-owned temporary file must remain"),
        b"unrelated caller-owned data"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output_path).unwrap()).unwrap();
    assert!(manifest["count"].as_u64().unwrap() > 0);
}

#[test]
fn generate_schema_rejects_malformed_or_duplicate_constructor_ids_without_replacing_output() {
    let dir = tempfile::tempdir().unwrap();
    let schema = dir.path().join("input.tl");
    let output_path = dir.path().join("manifest.json");
    for source in [
        "valid#01 = X;\ninvalid#not_hex = X;\n",
        "valid#01 = X;\nother#01 = X;\n",
        "valid#01 = X;\ntruncated#02 = X\n",
    ] {
        std::fs::write(&schema, source).unwrap();
        std::fs::write(&output_path, b"previous manifest").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_generate-schema"))
            .arg("--schema")
            .arg(&schema)
            .arg("--out")
            .arg(&output_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(std::fs::read(&output_path).unwrap(), b"previous manifest");
    }
}
