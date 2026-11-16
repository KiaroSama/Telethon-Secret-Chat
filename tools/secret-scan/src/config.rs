//! Four audited public-data exemptions; the scanned workspace cannot extend them.
//!
//! The two 256-byte fixture keys are arithmetic sequences, never authorization
//! data. The other-key receive fixture is ciphertext produced with synthetic
//! material. The final value is the SHA-256 of the unchanged official client
//! source at e61cf8585e1e9969ab81eda9fb4725a1500e2d86, recorded in both the
//! original-source and distributed-source manifests.
//!
//! Require the built-in generic-api-key rule, exact relative paths, and the
//! entire JSON field/value line. Changing a value, copying it elsewhere, or
//! adding another credential to either file must remain detectable.

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
    config
}
