//! Every original service sentence and channel/group variation.
#![allow(clippy::unwrap_used)]
#[path = "rust_export_support/desktop744.rs"]
mod current;
use kiaro_secret_chat::export::{
    format::DateContext, html_service, html_text::PeersMap, model_dialogs::DialogInfo,
    model_message::Message,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{fs, path::Path};
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn run(name: &str) {
    let case: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_service")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    let traces = case["traces"].as_array().unwrap();
    assert!(!traces.is_empty());
    for trace in traces {
        let args = trace["args"].as_array().unwrap();
        let dates = DateContext::named(trace["timezone"].as_str().unwrap()).unwrap();
        let function = trace["function"].as_str().unwrap();
        let result = match function {
            "html_service.qt_number" => html_service::qt_number(args[0].as_f64().unwrap()),
            "html_service.service_text" => {
                let message: Message = decode(&args[0]);
                let dialog: DialogInfo = decode(&args[1]);
                let peers: PeersMap = decode(&args[2]);
                if name == "14702609bbab7c5a.json" {
                    let mut minute = message.clone();
                    let Some(kiaro_secret_chat::export::model_actions::Action::SetMessagesTTL(ttl)) =
                        &mut minute.action.content
                    else {
                        panic!("TTL fixture")
                    };
                    ttl.period = 60;
                    let text = html_service::service_text(
                        &minute,
                        &dialog,
                        &peers,
                        "",
                        "",
                        str::to_owned,
                        &dates,
                    )
                    .unwrap();
                    assert_eq!(
                        text,
                        b"Ann Lee has set messages to auto-delete in 60 seconds"
                    );
                }
                let prefix = trace["kwargs"]["link_prefix"].as_str().unwrap();
                let suffix = trace["kwargs"]["link_suffix"].as_str().unwrap();
                String::from_utf8(
                    html_service::service_text(
                        &message,
                        &dialog,
                        &peers,
                        args[3].as_str().unwrap(),
                        args[4].as_str().unwrap(),
                        |text| format!("{prefix}{text}{suffix}"),
                        &dates,
                    )
                    .unwrap(),
                )
                .unwrap()
            }
            _ => panic!("Unregistered service function: {function}"),
        };
        assert_eq!(
            Value::String(result),
            trace
                .get("current_result")
                .unwrap_or(&trace["result"])
                .clone(),
            "{}: {function}",
            case["legacy_case"]
        );
        assert_eq!(
            trace["args"], trace["after"],
            "Pure service function mutated input"
        );
    }
}
#[test]
fn service_abbfb11d8c98b89f() {
    run("abbfb11d8c98b89f.json");
}
#[test]
fn service_41ca65169b8afccd() {
    run("41ca65169b8afccd.json");
}
#[test]
fn service_233c669a98753ccb() {
    run("233c669a98753ccb.json");
}
#[test]
fn service_cd750d1312cde1cb() {
    run("cd750d1312cde1cb.json");
}
#[test]
fn service_9be7f35a5475f9ee() {
    run("9be7f35a5475f9ee.json");
}
#[test]
fn service_66aee89f69e0f942() {
    run("66aee89f69e0f942.json");
}
#[test]
fn service_449fee8aa57fcbb7() {
    run("449fee8aa57fcbb7.json");
}
#[test]
fn service_65996091438f3e4d() {
    run("65996091438f3e4d.json");
}
#[test]
fn service_3589cce85895fda8() {
    run("3589cce85895fda8.json");
}
#[test]
fn service_ee640155551100f9() {
    run("ee640155551100f9.json");
}
#[test]
fn service_c796e2ac82078078() {
    run("c796e2ac82078078.json");
}
#[test]
fn service_e78c2f6b7d4dda5d() {
    run("e78c2f6b7d4dda5d.json");
}
#[test]
fn service_c428e5b6eb634215() {
    run("c428e5b6eb634215.json");
}
#[test]
fn service_16a6da3fb8298a46() {
    run("16a6da3fb8298a46.json");
}
#[test]
fn service_dd88b2f8af90753e() {
    run("dd88b2f8af90753e.json");
}
#[test]
fn service_61eb07da28a90975() {
    run("61eb07da28a90975.json");
}
#[test]
fn service_ee4042f6ed1d07bb() {
    run("ee4042f6ed1d07bb.json");
}
#[test]
fn service_c3bcee5f95c50426() {
    run("c3bcee5f95c50426.json");
}
#[test]
fn service_2c66edf42e416cd9() {
    let case: Value = serde_json::from_str(include_str!(
        "fixtures/tdexport_service/2c66edf42e416cd9.json"
    ))
    .unwrap();
    let args = case["traces"][0]["args"].as_array().unwrap();
    let mut message: Message = decode(&args[0]);
    let Some(kiaro_secret_chat::export::model_actions::Action::BotAllowed(bot)) =
        &mut message.action.content
    else {
        panic!("bot-allowed fixture")
    };
    bot.app.clear();
    bot.domain = "example<site>".into();
    let domain = html_service::service_text(
        &message,
        &decode(&args[1]),
        &decode(&args[2]),
        "",
        "",
        str::to_owned,
        &DateContext::utc(),
    )
    .unwrap();
    assert_eq!(
        domain,
        b"You allowed this bot to message you when you logged in on example&lt;site&gt;"
    );
    run("2c66edf42e416cd9.json");
}
#[test]
fn service_a3aff938d51aa7de() {
    run("a3aff938d51aa7de.json");
}
#[test]
fn service_d91fc742100a8b6f() {
    run("d91fc742100a8b6f.json");
}
#[test]
fn service_655633633238c7df() {
    run("655633633238c7df.json");
}
#[test]
fn service_316cef1be6f6d3a4() {
    run("316cef1be6f6d3a4.json");
}
#[test]
fn service_5dfd28bda3962db3() {
    run("5dfd28bda3962db3.json");
}
#[test]
fn service_a245773dba59cd94() {
    run("a245773dba59cd94.json");
}
#[test]
fn service_02f5acdc982bc341() {
    run("02f5acdc982bc341.json");
}
#[test]
fn service_705cd0676527afe4() {
    run("705cd0676527afe4.json");
}
#[test]
fn service_14702609bbab7c5a() {
    run("14702609bbab7c5a.json");
}
#[test]
fn service_99be3f4acf67ae1c() {
    run("99be3f4acf67ae1c.json");
}
#[test]
fn service_1c69d697441f1d7b() {
    run("1c69d697441f1d7b.json");
}
#[test]
fn service_faf3e03907fa4607() {
    run("faf3e03907fa4607.json");
}
#[test]
fn service_27e4ada2cc18019b() {
    run("27e4ada2cc18019b.json");
}
#[test]
fn service_8d1431a99f3fca9c() {
    run("8d1431a99f3fca9c.json");
}
#[test]
fn service_6a2a09138885a43d() {
    run("6a2a09138885a43d.json");
}
#[test]
fn service_d9840e6341018329() {
    run("d9840e6341018329.json");
}
#[test]
fn service_af7cdab0236ae6e4() {
    run("af7cdab0236ae6e4.json");
}
#[test]
fn service_2ecf610c76edb58f() {
    run("2ecf610c76edb58f.json");
}
#[test]
fn service_f791095138e85a78() {
    run("f791095138e85a78.json");
}
#[test]
fn service_045cc8833e410e9d() {
    run("045cc8833e410e9d.json");
}
#[test]
fn service_d8e0b8f5ab0ae2d3() {
    run("d8e0b8f5ab0ae2d3.json");
}
#[test]
fn service_709cfd1debbda592() {
    run("709cfd1debbda592.json");
}
#[test]
fn service_153961b984df73a8() {
    run("153961b984df73a8.json");
}
#[test]
fn service_39831a8a6d054c08() {
    run("39831a8a6d054c08.json");
}
#[test]
fn service_17d6a5e245688d18() {
    run("17d6a5e245688d18.json");
}
#[test]
fn service_f23730e8132d52b2() {
    run("f23730e8132d52b2.json");
}
#[test]
fn service_5ce1328b639a9efd() {
    run("5ce1328b639a9efd.json");
}
#[test]
fn service_fe7e5bed67b57d27() {
    run("fe7e5bed67b57d27.json");
}
#[test]
fn service_68429557c337ec1a() {
    run("68429557c337ec1a.json");
}
#[test]
fn service_8b38cecb94bc8c62() {
    run("8b38cecb94bc8c62.json");
}
#[test]
fn service_7cfed3ef6bd62a42() {
    run("7cfed3ef6bd62a42.json");
}
#[test]
fn service_b3d76f9d927785ad() {
    run("b3d76f9d927785ad.json");
}
#[test]
fn service_81b2aa3430cefc69() {
    run("81b2aa3430cefc69.json");
}
#[test]
fn service_6222edc8e8eee923() {
    run("6222edc8e8eee923.json");
}
#[test]
fn service_db894796ed152251() {
    run("db894796ed152251.json");
}
#[test]
fn service_0780531b838a9ea8() {
    run("0780531b838a9ea8.json");
}
#[test]
fn service_06abcf4a6ee8e888() {
    run("06abcf4a6ee8e888.json");
}
#[test]
fn service_37be1f55cfb25822() {
    run("37be1f55cfb25822.json");
}
#[test]
fn service_7659af3429745120() {
    run("7659af3429745120.json");
}
#[test]
fn service_635667953e7fdb1f() {
    run("635667953e7fdb1f.json");
}
#[test]
fn service_0634ed5b9ef3975d() {
    run("0634ed5b9ef3975d.json");
}
#[test]
fn service_ca469d6184de0796() {
    run("ca469d6184de0796.json");
}
#[test]
fn service_6193415d6b0d1653() {
    run("6193415d6b0d1653.json");
}
#[test]
fn service_845d996e84ae4847() {
    run("845d996e84ae4847.json");
}
#[test]
fn service_a934e0d3e072a00e() {
    run("a934e0d3e072a00e.json");
}
#[test]
fn service_1b7ec52e8d6d9f2d() {
    run("1b7ec52e8d6d9f2d.json");
}
#[test]
fn service_87080b2156cbb956() {
    current::html_safety();
    run("87080b2156cbb956.json");
}
#[test]
fn service_4afe8b2e2d5d6941() {
    run("4afe8b2e2d5d6941.json");
}
#[test]
fn service_1c6b123c8c7aaa56() {
    run("1c6b123c8c7aaa56.json");
}
#[test]
fn service_84df9dc10f304ae1() {
    run("84df9dc10f304ae1.json");
}
#[test]
fn service_99ab1126eddbfee4() {
    run("99ab1126eddbfee4.json");
}
#[test]
fn service_2e5a4417fb708fbe() {
    run("2e5a4417fb708fbe.json");
}
#[test]
fn service_6ef901323f91f042() {
    run("6ef901323f91f042.json");
}
#[test]
fn service_f1909ec78d6c56ac() {
    run("f1909ec78d6c56ac.json");
}
#[test]
fn service_0929f8b0851dd76b() {
    run("0929f8b0851dd76b.json");
}
#[test]
fn service_12ced06b82ab1268() {
    run("12ced06b82ab1268.json");
}
#[test]
fn service_daf3235c9ce10523() {
    run("daf3235c9ce10523.json");
}
#[test]
fn service_8f2ff08d6a924966() {
    run("8f2ff08d6a924966.json");
}
#[test]
fn service_61ba8e9ef69c2a22() {
    run("61ba8e9ef69c2a22.json");
}
#[test]
fn service_409a2ed0a12ee216() {
    run("409a2ed0a12ee216.json");
}
#[test]
fn service_811dc2dfcce3ce37() {
    run("811dc2dfcce3ce37.json");
}
#[test]
fn service_5c35856837f9d841() {
    run("5c35856837f9d841.json");
}
#[test]
fn service_d288b4889b7db330() {
    run("d288b4889b7db330.json");
}
#[test]
fn service_113c4510acdcc166() {
    run("113c4510acdcc166.json");
}
#[test]
fn service_83bde863770af9dd() {
    run("83bde863770af9dd.json");
}
#[test]
fn service_1aedba18ccaa50d1() {
    run("1aedba18ccaa50d1.json");
}
#[test]
fn service_773eeaf68d39bed3() {
    run("773eeaf68d39bed3.json");
}
#[test]
fn service_79e2eb81ac7532d7() {
    run("79e2eb81ac7532d7.json");
}
#[test]
fn service_c27628c515c6d4f3() {
    run("c27628c515c6d4f3.json");
}
#[test]
fn service_e6956136f2383762() {
    run("e6956136f2383762.json");
}
#[test]
fn service_e5ac8d7393f1c8a2() {
    run("e5ac8d7393f1c8a2.json");
}
#[test]
fn service_f72d35176080059f() {
    // Keep the capture immutable and separately distinguish sender from recipient.
    let case: Value = serde_json::from_str(include_str!(
        "fixtures/tdexport_service/f72d35176080059f.json"
    ))
    .unwrap();
    let args = case["traces"][0]["args"].as_array().unwrap();
    let mut message: Message = decode(&args[0]);
    let dialog: DialogInfo = decode(&args[1]);
    let peers: PeersMap = decode(&args[2]);
    message.from_id = 9;
    assert_ne!(message.from_id, dialog.peer_id);
    let text = html_service::service_text(
        &message,
        &dialog,
        &peers,
        "https://t.me/",
        "",
        str::to_owned,
        &DateContext::utc(),
    )
    .unwrap();
    assert_eq!(text, b"You refunded 20 Stars for 2 messages to Ann Lee");
    assert_eq!(case["traces"][0]["args"][0]["from_id"], 5);

    run("f72d35176080059f.json");
}
#[test]
fn service_60e79a3da06ec446() {
    run("60e79a3da06ec446.json");
}
#[test]
fn service_1651e124a0afba46() {
    run("1651e124a0afba46.json");
}
