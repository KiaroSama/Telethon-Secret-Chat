//! Audited exceptions for the built-in generic-api-key rule.
//!
//! Current fixtures use deterministic test keys or synthetic ciphertext. The
//! grammers exception is a verified upstream source checksum. Historical
//! exceptions also pin the commit, so the scanner never needs a copy of the
//! retired throwaway chat key.

const OTHER_FRAME: &str = "c04b048704bb081f302bbba287d470c3b8305ac7899ee2e1b483bab5df5e53b03ba723700083ac8ffc789ec33bbfd5e405ddef50ec31afbd3ca82a20bbcde57c8e3a70d8325ee4dce8c8090529a892c9a7db513d95c8b27011b5b8954bdf5a8c2d32cd4dec932d98";
const CLIENT_SOURCE_SHA256: &str =
    "d025abd234d75cae8d3acd0c4b1f782e794999b4eaa59dc1fc97d2f022431d83";

fn arithmetic_bytes(step: usize, start: usize) -> String {
    (0..256)
        .map(|index| format!("{:02x}", (index * step + start) % 256))
        .collect()
}

pub fn configuration() -> String {
    let mut config =
        "[extend]\nuseDefault = true\n\n[[rules]]\nid = \"generic-api-key\"\n".to_owned();
    let exemptions = [
        (
            r"^tests/fixtures/receive-oracle\.json$",
            "key",
            arithmetic_bytes(11, 5),
            "Public deterministic MTProto frame test key",
        ),
        (
            r"^tests/fixtures/receive-oracle\.json$",
            "other_key",
            OTHER_FRAME.to_owned(),
            "Public synthetic mismatched-key ciphertext frame",
        ),
        (
            r"^tests/fixtures/kdf-telethon-reference\.json$",
            "key",
            arithmetic_bytes(7, 13),
            "Public deterministic independent KDF test key",
        ),
        (
            r"^vendor/grammers/(upstream-hashes|local-patches)\.json$",
            r"grammers-client/src/client/auth\.rs",
            CLIENT_SOURCE_SHA256.to_owned(),
            "Verified official client source SHA-256 checksum",
        ),
    ];
    for (path, field, value, description) in exemptions {
        config.push_str(&format!(
            r#"
[[rules.allowlists]]
description = "{description}"
condition = "AND"
regexTarget = "line"
paths = ['''{path}''']
regexes = ['''^\s*"{field}"\s*:\s*"{value}",?\s*$''']
"#
        ));
    }
    for (commit, path, line, description) in [
        (
            "675c57e597af95b5d12acae9fbd964275e53ec68",
            r"^telethon_secret_chat/ingress_provenance\.py$",
            r#"^\s*"telethon/client/auth\.py": "[0-9a-f]{64}",\s*$"#,
            "Historical patched Telethon source checksum",
        ),
        (
            "b1838488979293f33c93f56470c61ae63aa4056a",
            r"^tests/fixtures/tdexport-source\.json$",
            r#"^\s*"secret_saved\.py": "[0-9a-f]{64}",?\s*$"#,
            "Historical vendored exporter source checksum",
        ),
        (
            "723585f8d65b0c7c62c0177f4f683f3f1da36bf9",
            r"^tests/vectors/fixtures/official-client-2026-09-29\.json$",
            r#"^\s*"key": "[0-9a-f]{512}",\s*$"#,
            "Original offline vector from a throwaway chat",
        ),
    ] {
        config.push_str(&format!(
            r#"
[[rules.allowlists]]
description = "{description}"
condition = "AND"
regexTarget = "line"
commits = ["{commit}"]
paths = ['''{path}''']
regexes = ['''{line}''']
"#
        ));
    }
    config
}
