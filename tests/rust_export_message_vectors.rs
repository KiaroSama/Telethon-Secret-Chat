#![allow(clippy::unwrap_used)]
//! Full Desktop bubble markup, grouping, forwarding, services and IO scenarios.
use image::GenericImageView;
use kiaro_secret_chat::export::{
    format::DateContext,
    html_message::MessageInfo,
    html_service,
    html_text::{HtmlContext, WrapBase},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn encode<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}
fn link(kind: &str, id: i64, body: &str) -> String {
    match kind {
        "bracket" => format!("[{id}:{body}]"),
        "html" => format!(
            "<a href=\"#go_to_message{id}\" onclick=\"return GoToMessage({id})\">{body}</a>"
        ),
        _ => panic!("Unknown test callback"),
    }
}
fn files(root: &Path, folder: &Path, out: &mut BTreeMap<String, Value>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(root, &path, out);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|p| p.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            let size = fs::read(&path)
                .ok()
                .and_then(|b| image::load_from_memory(&b).ok())
                .map(|i| i.dimensions());
            out.insert(name, json!({"size":size}));
        }
    }
}
fn run(name: &str) {
    let case: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_message")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    let traces = case["traces"].as_array().unwrap();
    assert!(!traces.is_empty());
    for trace in traces {
        let args = trace["args"].as_array().unwrap();
        let mut after = args.clone();
        let function = trace["function"].as_str().unwrap();
        let dates = DateContext::named(trace["timezone"].as_str().unwrap_or("UTC")).unwrap();
        let folder = tempfile::tempdir().unwrap();
        if let Some(before) = trace["files_before"].as_object() {
            for (name, data) in before {
                let path = folder.path().join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, decode::<Vec<u8>>(data)).unwrap();
            }
        }
        let result = if function == "html_service.service_text" {
            let prefix = trace["kwargs"]["link_prefix"].as_str().unwrap();
            let suffix = trace["kwargs"]["link_suffix"].as_str().unwrap();
            encode(text(
                html_service::service_text(
                    &decode(&args[0]),
                    &decode(&args[1]),
                    &decode(&args[2]),
                    args[3].as_str().unwrap(),
                    args[4].as_str().unwrap(),
                    |body| format!("{prefix}{body}{suffix}"),
                    &dates,
                )
                .unwrap(),
            ))
        } else {
            let mut wrap = WrapBase {
                base: args[0]["base"].as_str().unwrap().into(),
                context: decode::<HtmlContext>(&args[0]["context"]),
            };
            let kind = trace["link_kind"].as_str().unwrap_or("");
            let output = match function {
                "html_message.MessageMixin.push_message" => {
                    let (info, bytes) = wrap
                        .push_message(
                            &decode(&args[1]),
                            decode::<Option<MessageInfo>>(&args[2]).as_ref(),
                            &decode(&args[3]),
                            folder.path(),
                            &decode(&args[5]),
                            args[6].as_str().unwrap(),
                            |id, body| link(kind, id, body),
                            &dates,
                        )
                        .unwrap();
                    encode((info, text(bytes)))
                }
                "html_message.MessageMixin.message_needs_wrap" => encode(
                    wrap.message_needs_wrap(
                        &decode(&args[1]),
                        decode::<Option<MessageInfo>>(&args[2]).as_ref(),
                        &dates,
                    )
                    .unwrap(),
                ),
                "html_message.MessageMixin.forwarded_needs_wrap" => encode(
                    wrap.forwarded_needs_wrap(
                        &decode(&args[1]),
                        decode::<Option<MessageInfo>>(&args[2]).as_ref(),
                        &dates,
                    )
                    .unwrap(),
                ),
                "html_media_extra.ExtraMediaMixin.push_poll" => encode(text(
                    wrap.push_poll(
                        &decode(&args[1]),
                        args[2].as_str().unwrap(),
                        args[3].as_str().unwrap(),
                    )
                    .unwrap(),
                )),
                "html_media_extra.ExtraMediaMixin.push_todo_list" => encode(text(
                    wrap.push_todo_list(
                        &decode(&args[1]),
                        args[2].as_str().unwrap(),
                        args[3].as_str().unwrap(),
                    )
                    .unwrap(),
                )),
                "html_media_extra.ExtraMediaMixin.push_giveaway_start" => encode(
                    wrap.push_giveaway_start(&decode(&args[1]), &decode(&args[2]), &dates)
                        .unwrap(),
                ),
                "html_media_extra.ExtraMediaMixin.push_giveaway_results" => encode(
                    wrap.push_giveaway_results(&decode(&args[1]), &decode(&args[2]), |id, body| {
                        link(kind, id, body)
                    })
                    .unwrap(),
                ),
                _ => panic!("Unregistered native message function: {function}"),
            };
            after[0] = json!({"base":wrap.base,"context":wrap.context});
            output
        };
        assert_eq!(
            result, trace["result"],
            "{}: {function} output",
            case["legacy_case"]
        );
        assert_eq!(
            encode(after),
            trace["after"],
            "{}: {function} mutation",
            case["legacy_case"]
        );
        if let Some(before) = trace["files_before"].as_object() {
            for (name, data) in before {
                assert_eq!(
                    fs::read(folder.path().join(name)).unwrap(),
                    decode::<Vec<u8>>(data),
                    "Source media changed"
                );
            }
            let mut actual = BTreeMap::new();
            files(folder.path(), folder.path(), &mut actual);
            assert_eq!(
                encode(actual),
                trace["files_after"],
                "{}: {function} filesystem",
                case["legacy_case"]
            );
        }
    }
}
#[test]
fn message_2efd95de6a5d0d21() {
    run("2efd95de6a5d0d21.json");
}
#[test]
fn message_88cd2a2f0993c37e() {
    run("88cd2a2f0993c37e.json");
}
#[test]
fn message_fae885b6129346ac() {
    run("fae885b6129346ac.json");
}
#[test]
fn message_8fac8e3e7f235051() {
    run("8fac8e3e7f235051.json");
}
#[test]
fn message_52c2e50972c1c6f0() {
    run("52c2e50972c1c6f0.json");
}
#[test]
fn message_e97a8b34b9430ddc() {
    run("e97a8b34b9430ddc.json");
}
#[test]
fn message_bb3fd35dd33eabc4() {
    run("bb3fd35dd33eabc4.json");
}
#[test]
fn message_ec480a9ee74839e6() {
    run("ec480a9ee74839e6.json");
}
#[test]
fn message_53e85ce29e495332() {
    run("53e85ce29e495332.json");
}
#[test]
fn message_a4a333e42e84ae3e() {
    run("a4a333e42e84ae3e.json");
}
#[test]
fn message_39e9bdb9504ec728() {
    run("39e9bdb9504ec728.json");
}
#[test]
fn message_49c02e09d37b9337() {
    run("49c02e09d37b9337.json");
}
#[test]
fn message_a05bc1641f5f1db6() {
    run("a05bc1641f5f1db6.json");
}
#[test]
fn message_7d629a020dde1a15() {
    run("7d629a020dde1a15.json");
}
#[test]
fn message_c3676f6a60adafd5() {
    run("c3676f6a60adafd5.json");
}
#[test]
fn message_fa7f9b1e36ce461b() {
    run("fa7f9b1e36ce461b.json");
}
#[test]
fn message_9fec4bcda4a1b537() {
    run("9fec4bcda4a1b537.json");
}
#[test]
fn message_f6606a092ef95aa3() {
    run("f6606a092ef95aa3.json");
}
#[test]
fn message_d2b4426a850d5758() {
    run("d2b4426a850d5758.json");
}
#[test]
fn message_afec0eb985974027() {
    run("afec0eb985974027.json");
}
#[test]
fn message_020ad34c3e47d9a7() {
    run("020ad34c3e47d9a7.json");
}
#[test]
fn message_5629352d37eb721d() {
    run("5629352d37eb721d.json");
}
#[test]
fn message_120a6fa12f9085a2() {
    run("120a6fa12f9085a2.json");
}
#[test]
fn message_1dcf9634750d6ef2() {
    run("1dcf9634750d6ef2.json");
}
#[test]
fn message_3e5d6e43c50fc4e1() {
    run("3e5d6e43c50fc4e1.json");
}
#[test]
fn message_12a8ba227ca031f0() {
    run("12a8ba227ca031f0.json");
}
#[test]
fn message_cd6acb4119134434() {
    run("cd6acb4119134434.json");
}
#[test]
fn message_ab9b54a2ac84efdf() {
    run("ab9b54a2ac84efdf.json");
}
#[test]
fn message_2e52c6da1c81ad9f() {
    run("2e52c6da1c81ad9f.json");
}
