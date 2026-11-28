//! Execute copied native tools after their original source stage is deleted.
#![allow(clippy::unwrap_used)]
use super::{cargo, hashes, run};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, process::Command};
pub fn verify(stage: &Path, target: &Path, empty: &Path, upstream: &Path) -> bool {
    let mut expected = BTreeMap::new();
    hashes(
        &stage.join("vendor/grammers"),
        &stage.join("vendor/grammers"),
        &mut expected,
    );
    run(cargo(&stage.join("Cargo.toml"), target).args([
        "build",
        "--offline",
        "--locked",
        "--bins",
        "-j",
        "1",
    ]));
    let copied = empty.join("native tools");
    std::fs::create_dir(&copied).unwrap();
    for name in [
        "generate-schema",
        "prepare-sdk",
        "migration-audit",
        "matrix-evidence",
    ] {
        let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        std::fs::copy(target.join("debug").join(&filename), copied.join(filename)).unwrap();
    }
    std::fs::remove_dir_all(stage).unwrap();
    assert!(
        !stage.exists(),
        "the original compile-time source stage must be unavailable"
    );
    let executable = |name: &str| copied.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let output = empty.join("generated.rs");
    run(Command::new(executable("generate-schema"))
        .current_dir(empty)
        .arg("--output")
        .arg(&output));
    for _ in 0..2 {
        run(Command::new(executable("generate-schema"))
            .current_dir(empty)
            .arg("--output")
            .arg(&output)
            .arg("--check"));
    }
    let generated = std::fs::read_to_string(&output).unwrap();
    assert!(generated.contains("pub mod types") && generated.contains("pub mod enums"));
    let generated_crate = empty.join("generated crate");
    std::fs::create_dir(&generated_crate).unwrap();
    std::fs::write(generated_crate.join("Cargo.toml"),"[package]\nname='standalone-generated-schema'\nversion='0.0.0'\nedition='2024'\n[lib]\npath='lib.rs'\n[dependencies]\ngrammers-tl-types={git='https://codeberg.org/Lonami/grammers.git',rev='11e13a989a754f9cf01a798480a2e20ce9e5b1fd',version='=0.12.0'}\n[workspace]\n").unwrap();
    let mut library = String::from(
        "pub use grammers_tl_types::{Deserializable,Identifiable,RawVec,Serializable};\npub mod deserialize { pub use grammers_tl_types::deserialize::*; pub type Buffer<'a,'b>=&'a mut Cursor<'b>; }\n",
    );
    library.push_str(&generated);
    std::fs::write(generated_crate.join("lib.rs"), library).unwrap();
    run(cargo(&generated_crate.join("Cargo.toml"), target).args(["check", "--offline", "-j1"]));
    assert!(
        !stage.exists(),
        "compiling copied generator output must not restore its deleted source"
    );
    let prepared = empty.join("prepared SDK");
    run(Command::new(executable("prepare-sdk"))
        .current_dir(empty)
        .arg("--source")
        .arg(upstream)
        .arg("--output")
        .arg(&prepared));
    let mut actual = BTreeMap::new();
    hashes(&prepared, &prepared, &mut actual);
    assert_eq!(
        actual, expected,
        "embedded SDK supplement must reproduce every source byte"
    );
    let audit = run(Command::new(executable("migration-audit"))
        .current_dir(empty)
        .arg("--require-complete"));
    assert!(String::from_utf8(audit.stdout).unwrap().contains("1851"));
    // The evidence tool takes caller-provided records/source; this is a small
    // valid LLVM-format input, independent of the project's measured report.
    let source = empty.join("src");
    std::fs::create_dir(&source).unwrap();
    let source_file = source.join("lib.rs");
    std::fs::write(&source_file, "pub fn fixture() {}\n").unwrap();
    let metric = json!({"count":1,"covered":1,"percent":100.0});
    let summary = json!({"lines":metric,"regions":metric,"functions":metric});
    let value = json!({"type":"llvm.coverage.json.export","version":"3.1.0","data":[{"files":[{"filename":source_file.canonicalize().unwrap(),"summary":summary}],"totals":summary}]});
    let coverage = empty.join("caller-coverage.json");
    std::fs::write(&coverage, serde_json::to_vec(&value).unwrap()).unwrap();
    let result = run(Command::new(executable("matrix-evidence"))
        .current_dir(empty)
        .arg("--coverage")
        .arg(&coverage));
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("90% gate passed")
    );
    true
}
