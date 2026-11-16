#![allow(clippy::unwrap_used)]
//! Native HTML text and QUrl/SafeHref regression scenarios.
use kiaro_secret_chat::export::{
    format::{DateContext, iso_date_time},
    html_safe as safe, html_text as text,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{fs, path::Path};
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn encode<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}
fn run(name: &str) {
    let case: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_html_text")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    let traces = case["traces"].as_array().unwrap();
    assert!(!traces.is_empty());
    for trace in traces {
        let args = trace["args"].as_array().unwrap();
        let string = |i: usize| args[i].as_str().unwrap();
        let integer = |i: usize| args[i].as_i64().unwrap();
        let function = trace["function"].as_str().unwrap();
        let dates = DateContext::named(trace["timezone"].as_str().unwrap()).unwrap();
        let result = match function {
            "html_text.serialize_string" => encode(text::serialize_string(string(0))),
            "html_text.serialize_list" => {
                encode(text::serialize_list(&decode::<Vec<String>>(&args[0])))
            }
            "html_text.format_text" => encode(
                String::from_utf8(text::format_text(
                    &decode::<Vec<_>>(&args[0]),
                    string(1),
                    string(2),
                ))
                .unwrap(),
            ),
            "html_text.display_date" => {
                encode(text::display_date(integer(0), integer(1), &dates).unwrap())
            }
            "html_text.format_date_text" => {
                encode(text::format_date_text(integer(0), &dates).unwrap())
            }
            "html_text.format_time_text" => {
                encode(text::format_time_text(integer(0), &dates).unwrap())
            }
            "html_text.iso_date_time" => encode(iso_date_time(integer(0), &dates).unwrap()),
            "html_safe.safe_message_href" => encode(safe::safe_message_href(string(0))),
            "html_safe.safe_http_href" => encode(safe::safe_http_href(string(0))),
            "html_safe.safe_email_href" => encode(safe::safe_email_href(string(0))),
            "html_safe.safe_phone_href" => encode(safe::safe_phone_href(string(0))),
            "html_safe.safe_mention_href" => encode(safe::safe_mention_href(string(0), string(1))),
            "html_safe.is_hashtag_action" => encode(safe::is_hashtag_action(string(0))),
            "html_safe.is_bot_command_action" => encode(safe::is_bot_command_action(string(0))),
            "html_safe.is_cashtag_action" => encode(safe::is_cashtag_action(string(0))),
            "html_safe.safe_action_data" => encode(safe::safe_action_data(string(0), string(1))),
            "html_safe.safe_relative_emoji_href" => {
                encode(safe::safe_relative_emoji_href(string(0), string(1)))
            }
            "html_safe.anchor_token" => encode(safe::anchor_token(string(0))),
            "html_safe.qt_trimmed" => encode(safe::qt_trimmed(string(0))),
            _ => panic!("Unregistered native vector function: {function}"),
        };
        assert_eq!(
            result,
            trace
                .get("current_result")
                .unwrap_or(&trace["result"])
                .clone(),
            "{}: {function}",
            case["legacy_case"]
        );
        assert_eq!(trace["args"], trace["after"], "Pure function mutated input");
    }
}
#[test]
fn html_text_25eb644e961eae5c() {
    run("25eb644e961eae5c.json");
}
#[test]
fn html_text_8ddd7003b73beee3() {
    run("8ddd7003b73beee3.json");
}
#[test]
fn html_text_b63965213d0a696c() {
    run("b63965213d0a696c.json");
}
#[test]
fn html_text_ff7af11adc99934d() {
    run("ff7af11adc99934d.json");
}
#[test]
fn html_text_d54e448ca10a841e() {
    run("d54e448ca10a841e.json");
}
#[test]
fn html_text_9de42d88710b989e() {
    run("9de42d88710b989e.json");
}
#[test]
fn html_text_5c6ed4b3f62ae853() {
    run("5c6ed4b3f62ae853.json");
}
#[test]
fn html_text_b1659f68a46cf579() {
    run("b1659f68a46cf579.json");
}
#[test]
fn html_text_a4a892b31e96afef() {
    run("a4a892b31e96afef.json");
}
#[test]
fn html_text_befc6da0216d48c7() {
    run("befc6da0216d48c7.json");
}
#[test]
fn html_text_849e01d5ab447894() {
    run("849e01d5ab447894.json");
}
#[test]
fn html_text_9a0508a80aa057d5() {
    run("9a0508a80aa057d5.json");
}
#[test]
fn html_text_37ae10faa4374ad4() {
    run("37ae10faa4374ad4.json");
}
#[test]
fn html_text_7f812f1094e6ccf2() {
    run("7f812f1094e6ccf2.json");
}
#[test]
fn html_text_e5921fed337da94e() {
    run("e5921fed337da94e.json");
}
#[test]
fn html_text_f613244926e91b06() {
    run("f613244926e91b06.json");
}
#[test]
fn html_text_9ed37c9a8facd64d() {
    run("9ed37c9a8facd64d.json");
}
#[test]
fn html_text_8d71fa8900778701() {
    run("8d71fa8900778701.json");
}
#[test]
fn html_text_f0409edd204488bb() {
    run("f0409edd204488bb.json");
}
#[test]
fn html_text_d33edfd0dc723019() {
    run("d33edfd0dc723019.json");
}
#[test]
fn html_text_2d6eee6c13f638f9() {
    run("2d6eee6c13f638f9.json");
}
#[test]
fn html_text_94cb543d84f6a2a0() {
    run("94cb543d84f6a2a0.json");
}
#[test]
fn html_text_12617c64d5a42730() {
    run("12617c64d5a42730.json");
}
#[test]
fn html_text_5bd2c85bbea53c73() {
    run("5bd2c85bbea53c73.json");
}
#[test]
fn html_text_230b883385950ab4() {
    run("230b883385950ab4.json");
}
#[test]
fn html_text_59ba7530f9d773b1() {
    run("59ba7530f9d773b1.json");
}
#[test]
fn html_text_e319d043e4262402() {
    run("e319d043e4262402.json");
}
#[test]
fn html_text_65e499597ff86fce() {
    run("65e499597ff86fce.json");
}
#[test]
fn html_text_9d617476f0866690() {
    run("9d617476f0866690.json");
}
#[test]
fn html_text_18bed2f6af5cef88() {
    run("18bed2f6af5cef88.json");
}
#[test]
fn html_text_f55d4ad4c477135d() {
    run("f55d4ad4c477135d.json");
}
