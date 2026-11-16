//! Independent complete-output legacy oracles, including extra reference inputs.
#![allow(clippy::unwrap_used)]
#[path = "rust_export_support/desktop744.rs"]
mod current;
#[path = "rust_export_support/review_models.rs"]
mod review;
use kiaro_secret_chat::export::{
    format::DateContext, model::*, model_dialogs, model_document::*, model_message::*,
    model_peers::*, model_rich::*, parse_actions::*, parse_media::*, parse_rich::*,
    parse_rich_block::*, parse_rich_text::*, settings,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{fs, path::Path};
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn encode<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}
struct Visitor {
    events: Vec<Value>,
}
impl RichVisitor for Visitor {
    fn text(&mut self, text: &mut RichText) {
        self.events
            .push(json!({"kind":"text","value":encode(text)}));
    }
    fn photo(&mut self, photo: &mut Photo) {
        self.events
            .push(json!({"kind":"photo","value":encode(photo)}));
    }
    fn document(&mut self, document: &mut Document) {
        self.events
            .push(json!({"kind":"document","value":encode(document)}));
    }
}
fn run(name: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tdexport_models")
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
            "model.peer_from_user" => encode(peer_from_user(args[0].as_i64().unwrap())),
            "model.peer_from_chat" => encode(peer_from_chat(args[0].as_i64().unwrap())),
            "model.peer_from_channel" => encode(peer_from_channel(args[0].as_i64().unwrap())),
            "model.peer_to_bare_id" => encode(peer_to_bare_id(args[0].as_i64().unwrap())),
            "model.parse_text" => encode(parse_text(
                args[0].as_str().unwrap_or(""),
                args[1].as_array().map_or(&[], |v| v.as_slice()),
            )),
            "model.refresh_file_reference" => {
                let mut target: FileLocation = decode(&args[0]);
                let result = refresh_file_reference(&mut target, &decode(&args[1]));
                after[0] = encode(target);
                encode(result)
            }
            "model.parse_document"
            | "model_media.parse_media"
            | "model_actions.parse_service_action"
            | "model_rich_parse.parse_rich_message" => {
                let mut context: ParseMediaContext = decode(&args[0]);
                let folder = args[2].as_str().unwrap();
                let date = args[3].as_i64().unwrap();
                let result = match function {
                    "model.parse_document" => encode(
                        parse_document(&mut context, &args[1], folder, date, &dates).unwrap(),
                    ),
                    "model_media.parse_media" => {
                        encode(parse_media(&mut context, &args[1], folder, date, &dates).unwrap())
                    }
                    "model_actions.parse_service_action" => encode(
                        parse_service_action(&mut context, &args[1], folder, date, &dates).unwrap(),
                    ),
                    _ => encode(
                        parse_rich_message(&mut context, &args[1], folder, date, &dates).unwrap(),
                    ),
                };
                after[0] = encode(context);
                result
            }
            "model_message.parse_message" => {
                let mut context: ParseMediaContext = decode(&args[0]);
                let result =
                    parse_message(&mut context, &args[1], args[2].as_str().unwrap(), &dates)
                        .unwrap();
                after[0] = encode(context);
                encode(result)
            }
            "model_message.parse_messages_slice" => {
                let mut context: ParseMediaContext = decode(&args[0]);
                let result = parse_messages_slice(
                    &mut context,
                    args[1].as_array().unwrap(),
                    args[2].as_array().unwrap(),
                    args[3].as_array().unwrap(),
                    args[4].as_str().unwrap(),
                    &dates,
                )
                .unwrap();
                after[0] = encode(context);
                encode(result)
            }
            "model_message.adjust_migrate_message_ids" => {
                let mut slice: MessagesSlice = decode(&args[0]);
                adjust_migrate_message_ids(&mut slice);
                after[0] = encode(&slice);
                encode(slice)
            }
            "model_peers.parse_user" => encode(parse_user(&args[0])),
            "model_peers.parse_chat" => encode(parse_chat(&args[0])),
            "model_peers.parse_peers_lists" => encode(parse_peers_lists(
                args[0].as_array().unwrap(),
                args[1].as_array().unwrap(),
            )),
            "model_peers.empty_peer" => encode(empty_peer(args[0].as_i64().unwrap())),
            "model_rich_parse.parse_inline_button_action" => {
                encode(parse_inline_button_action(&args[0]))
            }
            "model_rich_parse.parse_rich_text" => encode(parse_rich_text(&args[0])),
            "model_rich_parse.parse_rich_block" => encode(parse_rich_block(&args[0])),
            "model_rich_parse.extract_full_rich_message" => encode(extract_full_rich_message(
                args[0].as_array().unwrap(),
                args[1].as_i64().unwrap(),
            )),
            "model_rich_parse.refresh_rich_message_file_reference" => encode(
                refresh_rich_message_file_reference(&decode(&args[0]), &decode(&args[1])),
            ),
            "model_rich_parse.visit_rich_message" => {
                let mut message: RichMessage = decode(&args[0]);
                let mut visitor = Visitor { events: vec![] };
                visit_rich_message(&mut message, &mut visitor);
                after[0] = encode(message);
                encode(visitor.events)
            }
            "model_dialogs.parse_dialogs_info_users" => encode(
                model_dialogs::parse_dialogs_info_users(&args[0], args[1].as_array().unwrap())
                    .unwrap(),
            ),
            "model_dialogs.parse_dialogs_info_chats" => encode(
                model_dialogs::parse_dialogs_info_chats(&args[0], args[1].as_array().unwrap())
                    .unwrap(),
            ),
            "model_dialogs.dialog_type_from_chat" => {
                encode(model_dialogs::dialog_type_from_chat(&decode(&args[0])))
            }
            "model_dialogs.add_migrate_from_slice" => {
                let mut to: model_dialogs::DialogInfo = decode(&args[0]);
                let result = model_dialogs::add_migrate_from_slice(
                    &mut to,
                    &decode(&args[1]),
                    args[2].as_i64().unwrap(),
                    args[3].as_i64().unwrap(),
                );
                after[0] = encode(to);
                encode(result)
            }
            "model_dialogs.finalize_dialogs_info" => {
                let mut info: model_dialogs::DialogsInfo = decode(&args[0]);
                let settings: settings::Settings = decode(&args[1]);
                model_dialogs::finalize_dialogs_info(&mut info, &settings).unwrap();
                after[0] = encode(info);
                Value::Null
            }
            _ => panic!("Unregistered native vector function: {function}"),
        };
        assert_eq!(
            result,
            trace
                .get("current_result")
                .unwrap_or(&trace["result"])
                .clone(),
            "{}: {function} result",
            case["legacy_case"]
        );
        assert_eq!(
            json!(after),
            trace
                .get("current_after")
                .unwrap_or(&trace["after"])
                .clone(),
            "{}: {function} mutation",
            case["legacy_case"]
        );
    }
}
#[test]
fn model_bff356ec02059d68() {
    run("bff356ec02059d68.json");
}
#[test]
fn model_655fe07b88eb3175() {
    run("655fe07b88eb3175.json");
}
#[test]
fn model_4daf171aea505ceb() {
    run("4daf171aea505ceb.json");
}
#[test]
fn model_8cd8560ef9e29a62() {
    run("8cd8560ef9e29a62.json");
}
#[test]
fn model_ad5b6b9e0425c28f() {
    run("ad5b6b9e0425c28f.json");
}
#[test]
fn model_89c6e0f9a15e519c() {
    run("89c6e0f9a15e519c.json");
}
#[test]
fn model_7d7bf05ab6fe7d17() {
    run("7d7bf05ab6fe7d17.json");
}
#[test]
fn model_8d54ce042ffe12f4() {
    run("8d54ce042ffe12f4.json");
}
#[test]
fn model_719e676e798a9a98() {
    run("719e676e798a9a98.json");
}
#[test]
fn model_65e699bdbf1379d7() {
    run("65e699bdbf1379d7.json");
}
#[test]
fn model_8385019b33c36810() {
    run("8385019b33c36810.json");
}
#[test]
fn model_13a6753afb3034c1() {
    run("13a6753afb3034c1.json");
}
#[test]
fn model_62f4199e5a15e833() {
    run("62f4199e5a15e833.json");
}
#[test]
fn model_59de3e8a1b0ba5c7() {
    run("59de3e8a1b0ba5c7.json");
}
#[test]
fn model_9053a6455b2dae38() {
    run("9053a6455b2dae38.json");
}
#[test]
fn model_bb4e155ce41756ad() {
    run("bb4e155ce41756ad.json");
}
#[test]
fn model_10b3a2ec1bc661c8() {
    review::messages();
    run("10b3a2ec1bc661c8.json");
}
#[test]
fn model_f8e4cf5da56aa47b() {
    run("f8e4cf5da56aa47b.json");
}
#[test]
fn model_515f4977525443ec() {
    run("515f4977525443ec.json");
}
#[test]
fn model_c91f19d9c58cb12d() {
    run("c91f19d9c58cb12d.json");
}
#[test]
fn model_2e6b799e2920fe88() {
    run("2e6b799e2920fe88.json");
}
#[test]
fn model_7744a731ddbbd0e5() {
    run("7744a731ddbbd0e5.json");
}
#[test]
fn model_c9717050fea911fd() {
    run("c9717050fea911fd.json");
}
#[test]
fn model_88eda2689e189964() {
    run("88eda2689e189964.json");
}
#[test]
fn model_3715503f67003e93() {
    run("3715503f67003e93.json");
}
#[test]
fn model_2f9857de6dc1ae65() {
    run("2f9857de6dc1ae65.json");
}
#[test]
fn model_9196c9b75f8d5b06() {
    current::model_actions();
    run("9196c9b75f8d5b06.json");
}
#[test]
fn model_cfe7cfab51b9f45a() {
    run("cfe7cfab51b9f45a.json");
}
#[test]
fn model_15e55983b4113bfa() {
    run("15e55983b4113bfa.json");
}
#[test]
fn model_9d3a539072dc5913() {
    run("9d3a539072dc5913.json");
}
#[test]
fn model_5b098f1cdc58d634() {
    run("5b098f1cdc58d634.json");
}
#[test]
fn model_7a47aa21bd71d27b() {
    run("7a47aa21bd71d27b.json");
}
#[test]
fn model_96e08469412e2fc4() {
    run("96e08469412e2fc4.json");
}
#[test]
fn model_a2dd71b5ac2b171d() {
    run("a2dd71b5ac2b171d.json");
}
#[test]
fn model_f5af8209577690e1() {
    run("f5af8209577690e1.json");
}
#[test]
fn model_b46ccb57a3ba4e3b() {
    run("b46ccb57a3ba4e3b.json");
}
#[test]
fn model_d9c6db5963520b70() {
    run("d9c6db5963520b70.json");
}
#[test]
fn model_6955a054e651d52f() {
    run("6955a054e651d52f.json");
}
#[test]
fn model_c575387cb5791a42() {
    run("c575387cb5791a42.json");
}
#[test]
fn model_237b753d6f52095b() {
    run("237b753d6f52095b.json");
}
#[test]
fn model_c1bd9155c0326367() {
    run("c1bd9155c0326367.json");
}
#[test]
fn model_59b55dca84919465() {
    run("59b55dca84919465.json");
}
#[test]
fn model_3e34f21ef1bf0c04() {
    run("3e34f21ef1bf0c04.json");
}
#[test]
fn model_3c815b24767d9c7d() {
    run("3c815b24767d9c7d.json");
}
#[test]
fn model_8e0db6e325970da1() {
    run("8e0db6e325970da1.json");
}
#[test]
fn model_0fe6251fe1424c76() {
    run("0fe6251fe1424c76.json");
}
#[test]
fn model_7c2217a7cf1ddad3() {
    run("7c2217a7cf1ddad3.json");
}
#[test]
fn model_18b72ffca8949394() {
    run("18b72ffca8949394.json");
}
#[test]
fn model_8976f2735406c90b() {
    run("8976f2735406c90b.json");
}
#[test]
fn model_1d596b00bda740d4() {
    run("1d596b00bda740d4.json");
}
#[test]
fn model_7278ba5dcc13c9da() {
    run("7278ba5dcc13c9da.json");
}
#[test]
fn model_c44b139bf4b5a3f7() {
    run("c44b139bf4b5a3f7.json");
}
#[test]
fn model_4b1d685cb442869d() {
    run("4b1d685cb442869d.json");
}
#[test]
fn model_4b17771649a09982() {
    run("4b17771649a09982.json");
}
#[test]
fn model_32a31ad29fb141b9() {
    run("32a31ad29fb141b9.json");
}
#[test]
fn model_44eb30c1b45a5456() {
    run("44eb30c1b45a5456.json");
}
#[test]
fn model_3b40c252f459e799() {
    run("3b40c252f459e799.json");
}
#[test]
fn model_967c5676c7394f7b() {
    run("967c5676c7394f7b.json");
}
#[test]
fn model_2c63c33c9314e5e4() {
    run("2c63c33c9314e5e4.json");
}
#[test]
fn model_b1a597b53a3b7944() {
    run("b1a597b53a3b7944.json");
}
#[test]
fn model_7619285fd74ebf23() {
    run("7619285fd74ebf23.json");
}
#[test]
fn model_378580111dbf485e() {
    run("378580111dbf485e.json");
}
#[test]
fn model_3e53d37a1697c488() {
    run("3e53d37a1697c488.json");
}
#[test]
fn model_0e73fc1848fd7be0() {
    run("0e73fc1848fd7be0.json");
}
#[test]
fn model_99179b916de8cbf3() {
    run("99179b916de8cbf3.json");
}
#[test]
fn model_6b08c79a9ae38db3() {
    run("6b08c79a9ae38db3.json");
}
#[test]
fn model_aee049296bbe1355() {
    run("aee049296bbe1355.json");
}
#[test]
fn model_fd8c8e5bc2e3bc53() {
    run("fd8c8e5bc2e3bc53.json");
}
#[test]
fn model_a31c8d3742d72526() {
    run("a31c8d3742d72526.json");
}
#[test]
fn model_1cd1f68b5000053e() {
    run("1cd1f68b5000053e.json");
}
#[test]
fn model_76465efe4be81ade() {
    run("76465efe4be81ade.json");
}
#[test]
fn model_433855ea10bb5607() {
    run("433855ea10bb5607.json");
}
#[test]
fn model_66487b9204960f37() {
    run("66487b9204960f37.json");
}
#[test]
fn model_720b1333296cbcd0() {
    run("720b1333296cbcd0.json");
}
#[test]
fn model_75a65a3ed09f49a2() {
    run("75a65a3ed09f49a2.json");
}
#[test]
fn model_43e938edb4e05734() {
    run("43e938edb4e05734.json");
}
#[test]
fn model_0a9161d3c82fdda9() {
    run("0a9161d3c82fdda9.json");
}
#[test]
fn model_a20e053980735631() {
    run("a20e053980735631.json");
}
#[test]
fn model_8ea3faa40a766eff() {
    run("8ea3faa40a766eff.json");
}
#[test]
fn model_d85f772d63ef8343() {
    run("d85f772d63ef8343.json");
}
#[test]
fn model_6166738fac2de839() {
    run("6166738fac2de839.json");
}
#[test]
fn model_08b5deac8c3022db() {
    run("08b5deac8c3022db.json");
}
#[test]
fn model_2f855141c4286496() {
    run("2f855141c4286496.json");
}
#[test]
fn model_249b97ae2fe62b8c() {
    run("249b97ae2fe62b8c.json");
}
#[test]
fn model_746a27690482b4c6() {
    run("746a27690482b4c6.json");
}
#[test]
fn model_b5fa38c166370be5() {
    run("b5fa38c166370be5.json");
}
#[test]
fn model_4c5fc83974d92b90() {
    run("4c5fc83974d92b90.json");
}
#[test]
fn model_e110529f3c1da518() {
    run("e110529f3c1da518.json");
}
#[test]
fn model_612374cc5c4dcfd4() {
    run("612374cc5c4dcfd4.json");
}
#[test]
fn model_ab5f03a64658ffb3() {
    run("ab5f03a64658ffb3.json");
}
#[test]
fn model_536eb4d4a92a708a() {
    run("536eb4d4a92a708a.json");
}
#[test]
fn model_9f3cd6cd6a898488() {
    run("9f3cd6cd6a898488.json");
}
#[test]
fn model_f6e2209b43953d5c() {
    run("f6e2209b43953d5c.json");
}
#[test]
fn model_2fb90460e6d94761() {
    run("2fb90460e6d94761.json");
}
#[test]
fn model_4d4bb95f65124fd6() {
    review::peers();
    run("4d4bb95f65124fd6.json");
}
#[test]
fn model_b397cc65a143d23c() {
    review::split_counts();
    run("b397cc65a143d23c.json");
}
#[test]
fn model_a5f3d33f879b1518() {
    run("a5f3d33f879b1518.json");
}
#[test]
fn model_e8d7cce77d69988d() {
    run("e8d7cce77d69988d.json");
}
