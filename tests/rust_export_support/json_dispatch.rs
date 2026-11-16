#![allow(clippy::unwrap_used)]
//! Independent complete byte outputs from each unchanged Desktop JSON scenario.
use kiaro_secret_chat::export::{
    format::DateContext,
    json_actions, json_message,
    json_rich::{self, RichSerializeContext},
    json_rich_block,
    json_serialize::{self as primitive, JsonContext},
    model_message::Message,
    model_rich::RichMessage,
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
fn data(value: &Value) -> Vec<u8> {
    if let Some(text) = value.as_str() {
        text.as_bytes().to_vec()
    } else if let Some(units) = value.get("utf16") {
        let units = units
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as u16)
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units).into_bytes()
    } else {
        decode(value)
    }
}
pub(super) fn run(name: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tdexport_json")
        .join(name);
    let case: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    let traces = case["traces"].as_array().unwrap();
    assert!(!traces.is_empty());
    for trace in traces {
        let args = trace["args"].as_array().unwrap();
        let mut after = args.clone();
        let function = trace["function"].as_str().unwrap();
        let dates = DateContext::named(trace["timezone"].as_str().unwrap()).unwrap();
        let result = match function {
            "json_serialize.serialize_string" => primitive::serialize_string(data(&args[0])),
            "json_serialize.string_allow_null" => {
                primitive::string_allow_null(args[0].as_str().unwrap())
            }
            "json_serialize.number" => {
                if let Some(n) = args[0].as_i64() {
                    primitive::number(n)
                } else {
                    primitive::number(args[0].as_f64().unwrap())
                }
            }
            "json_serialize.indentation" => primitive::indentation(&decode(&args[0])),
            "json_serialize.serialize_text" => {
                let mut context: JsonContext = decode(&args[0]);
                let result = primitive::serialize_text(
                    &mut context,
                    &decode::<Vec<_>>(&args[1]),
                    args.get(2).and_then(Value::as_bool).unwrap_or(false),
                );
                after[0] = encode(context);
                result
            }
            "json_message.serialize_message" => {
                let mut context: JsonContext = decode(&args[0]);
                let message: Message = decode(&args[1]);
                let result = json_message::serialize_message(
                    &mut context,
                    &message,
                    &decode(&args[2]),
                    args[3].as_str().unwrap(),
                    &dates,
                )
                .unwrap();
                after[0] = encode(context);
                result
            }
            "json_rich.serialize_rich_text" | "json_rich.serialize_rich_block" => {
                let mut context: JsonContext = decode(&args[0]["json"]);
                let message: RichMessage = decode(&args[0]["message"]);
                let mut rich = RichSerializeContext {
                    json: &mut context,
                    message: &message,
                    dates: &dates,
                };
                let result = if function == "json_rich.serialize_rich_text" {
                    json_rich::serialize_rich_text(&mut rich, &decode(&args[1])).unwrap()
                } else {
                    json_rich_block::serialize_rich_block(&mut rich, &decode(&args[1])).unwrap()
                };
                after[0]["json"] = encode(context);
                result
            }
            "json_rich.serialize_rich_message" => {
                let mut context: JsonContext = decode(&args[0]);
                let result =
                    json_rich::serialize_rich_message(&mut context, &decode(&args[1]), &dates)
                        .unwrap();
                after[0] = encode(context);
                result
            }
            "json_actions.serialize_todo_items" => {
                let mut context: JsonContext = decode(&args[0]);
                let result =
                    json_actions::serialize_todo_items(&mut context, &decode::<Vec<_>>(&args[1]));
                after[0] = encode(context);
                result
            }
            _ => panic!("Unregistered native vector function: {function}"),
        };
        assert_eq!(
            encode(result),
            trace["result"],
            "{}: {function} byte output",
            case["legacy_case"]
        );
        assert_eq!(
            encode(after),
            trace["after"],
            "{}: {function} mutation",
            case["legacy_case"]
        );
    }
}
