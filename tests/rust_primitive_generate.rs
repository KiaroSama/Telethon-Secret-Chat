#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::schema_generator::Generator;
use std::{fs, path::PathBuf};
const SCHEMA: &str = "decryptedMessageMediaEmpty#89f5c4a = DecryptedMessageMedia;\ndecryptedMessageActionNoop#a82fdd63 = DecryptedMessageAction;\n";
fn sandbox() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("end-to-end.tl");
    fs::write(&source, SCHEMA).unwrap();
    let target = folder.path().join("secret_tl.rs");
    (folder, source, target)
}
fn names(folder: &std::path::Path) -> Vec<String> {
    let mut result = fs::read_dir(folder)
        .unwrap()
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    result.sort();
    result
}
#[test]
fn formatter_failure_keeps_previous_module_and_cleans_staging() {
    let (folder, schema, target) = sandbox();
    fs::write(&target, b"// previous generated module\n").unwrap();
    let generator = Generator {
        formatter: folder.path().join("missing-rustfmt"),
    };
    assert!(generator.generate(&schema, &target, false).is_err());
    assert_eq!(
        fs::read(&target).unwrap(),
        b"// previous generated module\n"
    );
    assert_eq!(names(folder.path()), ["end-to-end.tl", "secret_tl.rs"]);
}
#[test]
fn check_matches_or_reports_drift_without_writing() {
    let (folder, schema, target) = sandbox();
    let generator = Generator::default();
    assert!(generator.generate(&schema, &target, false).unwrap());
    let generated = fs::read(&target).unwrap();
    assert!(generator.generate(&schema, &target, true).unwrap());
    let drift = [generated, b"// drift\n".to_vec()].concat();
    fs::write(&target, &drift).unwrap();
    assert!(!generator.generate(&schema, &target, true).unwrap());
    assert_eq!(fs::read(&target).unwrap(), drift);
    assert_eq!(names(folder.path()), ["end-to-end.tl", "secret_tl.rs"]);

    // The complete schema has multi-constructor getters; upstream uses a
    // randomized set for their order. Every fresh generation must still check.
    fs::write(&schema, include_str!("../schema/end-to-end.tl")).unwrap();
    assert!(generator.generate(&schema, &target, false).unwrap());
    let complete = fs::read(&target).unwrap();
    for _ in 0..16 {
        assert!(generator.generate(&schema, &target, true).unwrap());
        assert_eq!(fs::read(&target).unwrap(), complete);
    }
}
#[test]
fn missing_check_does_not_create_a_module() {
    let (folder, schema, target) = sandbox();
    assert!(
        !Generator::default()
            .generate(&schema, &target, true)
            .unwrap()
    );
    assert!(!target.exists());
    assert_eq!(names(folder.path()), ["end-to-end.tl"]);
}
