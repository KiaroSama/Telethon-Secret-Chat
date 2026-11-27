//! The Rust port of `rust/adapters/tool_check_test_matrix_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

const LIVE_SKIPS: [&str; 10] = [
    "tests.interop.test_live_roundtrip::test_both_ends_agree_on_the_key_fingerprint",
    "tests.interop.test_live_roundtrip::test_a_message_crosses_in_both_directions",
    "tests.interop.test_live_roundtrip::test_closing_here_is_seen_as_closed_here",
    "tests.interop.test_live_media::test_every_media_kind_is_sent_and_opens_on_the_far_side",
    "tests.interop.test_live_media::test_a_file_from_the_far_side_decrypts_and_is_written",
    "tests.interop.test_live_rekey::test_a_rekey_started_here_is_accepted_by_the_official_client",
    "tests.live_pair.test_pair_chat::test_one_chat_through_text_rekey_files_and_a_restart",
    "tests.live_pair.test_pair_close::test_a_close_reaches_the_other_side_on_its_next_update_sync",
    "tests.live_pair.test_pair_close::test_sending_into_a_chat_the_peer_closed_closes_it_here",
    "tests.live_pair.test_pair_delete::test_a_delete_for_both_sides_removes_the_chat_on_the_other_side",
];

fn declared_skips(leg: &str) -> std::collections::BTreeSet<&'static str> {
    let mut expected: std::collections::BTreeSet<_> = LIVE_SKIPS.into_iter().collect();
    if leg == "cases-windows-latest-py3.13" {
        expected.extend([
            "tests.unit.test_storage_contract::test_the_file_backend_is_owner_only",
            "tests.unit.test_guards::test_a_symlinked_store_is_refused",
            "tests.unit.test_guards::test_a_directory_fsync_failure_is_a_warning_not_a_rollback",
        ]);
    }
    if leg.starts_with("cases-ubuntu-") {
        expected.extend([
            "tests.test_tdexport_thumbs::test_thumb_is_mozjpeg_progressive_444_with_desktop_tables",
            "tests.test_tdexport_thumbs::test_thumb_carries_source_density_and_icc",
            "tests.test_tdexport_thumbs::test_unscaled_grayscale_stays_one_component",
            "tests.test_tdexport_thumbs::test_thumb_bytes_are_pinned",
            "tests.test_tdexport_thumbs::test_cmyk_thumbnail_uses_desktop_rgb_conversion[size0]",
            "tests.test_tdexport_thumbs::test_cmyk_thumbnail_uses_desktop_rgb_conversion[size1]",
            "tests.test_tdexport_thumbs::test_jpeg_comments_use_qt_key_order_and_utf8",
            "tests.test_tdexport_thumbs::test_non_srgb_thumbnail_matches_exact_qt_oracle",
        ]);
    }
    expected
}

fn pylist<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let names = values
        .map(|value| {
            let quote = if value.contains('\'') && !value.contains('"') {
                '"'
            } else {
                '\''
            };
            let value: String = value
                .chars()
                .map(|ch| match ch {
                    '\\' => "\\\\".into(),
                    '\n' => "\\n".into(),
                    '\r' => "\\r".into(),
                    '\t' => "\\t".into(),
                    c if c == quote => format!("\\{c}"),
                    c if c != ' '
                        && (c.is_whitespace()
                            || c.is_control()
                            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}')) =>
                    {
                        let n = u32::from(c);
                        if n <= 0xff {
                            format!("\\x{n:02x}")
                        } else {
                            format!("\\u{n:04x}")
                        }
                    }
                    c => c.to_string(),
                })
                .collect();
            format!("{quote}{value}{quote}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{names}]")
}

/// Port of `validate` (`rust/adapters/tool_check_test_matrix_adapter.py:13`).
pub fn validate(
    artifacts: serde_json::Value,
    legs: Vec<String>,
    count: i64,
) -> Result<serde_json::Value, String> {
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, BTreeSet};
    let rejected = |reason: String| json!({"exit":1,"output":[format!("ERROR: Matrix evidence rejected: {reason}")]});
    if legs.is_empty() {
        return Ok(json!({"exit":2,"output":[]}));
    }
    let expected: BTreeSet<_> = legs.iter().map(String::as_str).collect();
    if expected.len() != legs.len() {
        return Ok(rejected(
            "matrix leg names must be nonempty and unique".into(),
        ));
    }
    if i64::try_from(expected.len()).ok() != Some(count) {
        return Ok(rejected(format!(
            "expected {count} matrix legs, got {}",
            pylist(expected.iter().copied())
        )));
    }
    let mut records = BTreeMap::new();
    for (leg, xml) in artifacts.as_object().into_iter().flatten() {
        if !leg.is_empty()
            && leg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && let Some(xml) = xml.as_str()
        {
            records.insert(leg.as_str(), xml);
        }
    }
    let found: BTreeSet<_> = records.keys().copied().collect();
    if found != expected {
        return Ok(rejected(format!(
            "missing, duplicate, or unexpected matrix artifacts: expected {}, found {}",
            pylist(expected.iter().copied()),
            pylist(found.iter().copied())
        )));
    }
    let mut reference = None::<(&str, BTreeSet<String>)>;
    for (leg, xml) in records {
        let doc = match roxmltree::Document::parse(xml) {
            Ok(doc) => doc,
            Err(error) => {
                let pos = error.pos();
                return Ok(rejected(format!(
                    "syntax error: line {}, column {}",
                    pos.row,
                    pos.col.saturating_sub(1)
                )));
            }
        };
        let (mut cases, mut skipped) = (BTreeSet::new(), BTreeSet::new());
        for node in doc
            .descendants()
            .filter(|node| node.has_tag_name("testcase"))
        {
            let id = format!(
                "{}::{}",
                node.attribute("classname").unwrap_or_default(),
                node.attribute("name").unwrap_or_default()
            );
            if !cases.insert(id.clone()) {
                return Ok(rejected(format!("{leg}: duplicate testcase identity {id}")));
            }
            if node
                .children()
                .any(|n| n.has_tag_name("failure") || n.has_tag_name("error"))
            {
                return Ok(rejected(format!("{leg}: failing or errored test {id}")));
            }
            if node.children().any(|n| n.has_tag_name("skipped")) {
                skipped.insert(id);
            }
        }
        let declared = declared_skips(leg);
        let optional =
            "tests.vectors.test_official_client_frames::test_official_client_frames_replay_offline";
        let observed: BTreeSet<_> = skipped
            .iter()
            .map(String::as_str)
            .filter(|&s| s != optional)
            .collect();
        if declared != observed {
            return Ok(rejected(format!(
                "{leg}: skips differ from the {} declared ones: {}",
                declared.len(),
                pylist(observed.symmetric_difference(&declared).copied().take(20))
            )));
        }
        if cases == skipped {
            return Ok(rejected(format!("{leg}: executed no offline tests")));
        }
        if let Some((prev, ref previous)) = reference
            && previous != &cases
        {
            return Ok(rejected(format!(
                "{leg} and {prev} collected different testcase identities: {}",
                pylist(
                    cases
                        .symmetric_difference(previous)
                        .take(20)
                        .map(String::as_str)
                )
            )));
        }
        reference = Some((leg, cases));
    }
    let totals = reference.map_or(0, |(_, v)| v.len());
    let output: Vec<Value> = [
        format!(
            "WARNING: The {} live Telegram cases were intentionally NOT executed",
            LIVE_SKIPS.len()
        ),
        format!(
            "INFO: All {} matrix legs agree on {totals} testcases",
            expected.len()
        ),
    ]
    .into_iter()
    .map(Value::String)
    .collect();
    Ok(json!({"exit":0,"output":output}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_baseline_contract() -> Result<(), String> {
        let v = validate(serde_json::json!({}), vec!["a".into()], 1)?;
        assert_eq!(v["exit"], 1);
        assert!(
            v["output"][0]
                .as_str()
                .is_some_and(|s| s.contains("missing, duplicate, or unexpected"))
        );
        Ok(())
    }
}
