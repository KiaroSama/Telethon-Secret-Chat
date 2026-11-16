#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{Deserializable, Serializable, enums, types, wire};
use std::collections::{BTreeMap, BTreeSet};
pub fn published() -> BTreeMap<u32, String> {
    include_str!("../../schema/end-to-end.tl")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("//") || !line.ends_with(';') {
                return None;
            }
            let word = line.split_whitespace().next()?;
            let (name, id) = word.split_once('#')?;
            Some((u32::from_str_radix(id, 16).ok()?, name.into()))
        })
        .collect()
}
fn message() -> types::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1,
        ttl: 0,
        message: "hi".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
}
fn back(message: types::DecryptedMessage) -> types::DecryptedMessage {
    let e: enums::DecryptedMessage = message.into();
    let bytes = e.to_bytes();
    wire::validate(&bytes, "DecryptedMessage").unwrap();
    let enums::DecryptedMessage::Message(back) =
        enums::DecryptedMessage::from_bytes(&bytes).unwrap()
    else {
        panic!("wrong message family")
    };
    back
}
fn action(action: enums::DecryptedMessageAction) -> enums::DecryptedMessageAction {
    let bytes = action.to_bytes();
    wire::validate(&bytes, "DecryptedMessageAction").unwrap();
    enums::DecryptedMessageAction::from_bytes(&bytes).unwrap()
}
pub fn run(case: &str) {
    match case {
        "test_every_published_constructor_is_registered" => {
            for (id, name) in published() {
                let generated = kiaro_secret_chat::name_for_id(id);
                assert_eq!(generated.split("_legacy").next().unwrap(), name);
            }
        }
        "test_no_constructor_was_invented" => {
            let ast = syn::parse_file(include_str!(concat!(env!("OUT_DIR"), "/secret_common.rs")))
                .unwrap();
            let mut actual = BTreeSet::new();
            struct Ids<'a>(&'a mut BTreeSet<u32>);
            impl<'ast> syn::visit::Visit<'ast> for Ids<'_> {
                fn visit_arm(&mut self, arm: &'ast syn::Arm) {
                    if let syn::Pat::Lit(lit) = &arm.pat
                        && let syn::Lit::Int(n) = &lit.lit
                    {
                        self.0.insert(n.base10_parse().unwrap());
                    }
                    syn::visit::visit_arm(self, arm);
                }
            }
            syn::visit::Visit::visit_file(&mut Ids(&mut actual), &ast);
            actual.remove(&0x1cb5c415);
            assert_eq!(actual, published().keys().copied().collect());
        }
        "test_the_ids_the_protocol_reference_quotes_are_the_ones_generated" => {
            for (id, name) in [
                (0x1be31789, "decryptedMessageLayer"),
                (0x73164160, "decryptedMessageService"),
                (0xaa48327d, "decryptedMessageService"),
                (0xa1733aec, "decryptedMessageActionSetMessageTTL"),
                (0x0c4f40be, "decryptedMessageActionReadMessages"),
                (0x65614304, "decryptedMessageActionDeleteMessages"),
                (0x8ac1f475, "decryptedMessageActionScreenshotMessages"),
                (0x6719e45c, "decryptedMessageActionFlushHistory"),
                (0x511110b0, "decryptedMessageActionResend"),
                (0xf3048883, "decryptedMessageActionNotifyLayer"),
                (0xccb27641, "decryptedMessageActionTyping"),
                (0xf3c9611b, "decryptedMessageActionRequestKey"),
                (0x6fe1735b, "decryptedMessageActionAcceptKey"),
                (0xec2e0b9b, "decryptedMessageActionCommitKey"),
                (0xdd05ec6b, "decryptedMessageActionAbortKey"),
                (0xa82fdd63, "decryptedMessageActionNoop"),
            ] {
                assert_eq!(
                    kiaro_secret_chat::name_for_id(id)
                        .split("_legacy")
                        .next()
                        .unwrap(),
                    name
                );
            }
        }
        "test_all_thirteen_actions_exist" => {
            let schema = include_str!("../../schema/end-to-end.tl");
            assert_eq!(
                schema
                    .lines()
                    .filter(|l| l.trim_end().ends_with(" = DecryptedMessageAction;"))
                    .count(),
                13
            );
            let ast = syn::parse_file(include_str!(concat!(env!("OUT_DIR"), "/secret_enums.rs")))
                .unwrap();
            let n = ast
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Enum(e) if e.ident == "DecryptedMessageAction" => {
                        Some(e.variants.len())
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(n, 13);
        }
        "test_a_plain_field_object_round_trips" => {
            let a = types::DecryptedMessageActionSetMessageTtl { ttl_seconds: 3600 }.into();
            assert!(
                matches!(action(a),enums::DecryptedMessageAction::SetMessageTtl(a)if a.ttl_seconds==3600)
            );
        }
        "test_a_vector_of_longs_round_trips" => {
            let a = types::DecryptedMessageActionReadMessages {
                random_ids: vec![1, -2, 1i64 << 62],
            }
            .into();
            assert!(
                matches!(action(a),enums::DecryptedMessageAction::ReadMessages(a)if a.random_ids==[1,-2,1i64<<62])
            );
        }
        "test_an_empty_constructor_round_trips" => assert!(matches!(
            action(enums::DecryptedMessageAction::Noop),
            enums::DecryptedMessageAction::Noop
        )),
        "test_bytes_fields_survive_their_padding" => {
            let bytes: Vec<u8> = (0..512).map(|i| i as u8).collect();
            for n in [0, 1, 2, 3, 4, 7, 253, 254, 255, 260] {
                let a = types::DecryptedMessageActionRequestKey {
                    exchange_id: 7,
                    g_a: bytes[..n].to_vec(),
                }
                .into();
                assert!(
                    matches!(action(a),enums::DecryptedMessageAction::RequestKey(a)if a.g_a==bytes[..n])
                );
            }
        }
        "test_a_string_field_round_trips_as_utf8" => {
            let mut m = message();
            m.message = "ünïcödé ✓ 日本語".into();
            assert_eq!(back(m).message, "ünïcödé ✓ 日本語");
        }
        "test_an_absent_flag_field_stays_absent" => {
            let m = back(message());
            assert!(m.media.is_none() && m.entities.is_none() && m.grouped_id.is_none());
        }
        "test_a_present_flag_field_comes_back" => {
            let mut m = message();
            m.grouped_id = Some(99);
            m.reply_to_random_id = Some(5);
            let m = back(m);
            assert_eq!((m.grouped_id, m.reply_to_random_id), (Some(99), Some(5)));
        }
        "test_a_true_flag_is_carried_by_its_bit_alone" => {
            let plain = message();
            let mut flagged = plain.clone();
            flagged.no_webpage = true;
            let p: enums::DecryptedMessage = plain.clone().into();
            let f: enums::DecryptedMessage = flagged.clone().into();
            assert_eq!(p.to_bytes().len(), f.to_bytes().len());
            assert!(back(flagged).no_webpage);
            assert!(!back(plain).no_webpage);
        }
        "test_a_nested_object_round_trips" => {
            let inner = types::DecryptedMessageService {
                random_id: 42,
                action: types::DecryptedMessageActionNotifyLayer { layer: 144 }.into(),
            }
            .into();
            let w: enums::DecryptedMessageLayer = types::DecryptedMessageLayer {
                random_bytes: vec![1; 31],
                layer: 144,
                in_seq_no: 2,
                out_seq_no: 3,
                message: inner,
            }
            .into();
            let back = kiaro_secret_chat::framing::unwrap(&w.to_bytes()).unwrap();
            assert_eq!((back.layer, back.in_seq_no, back.out_seq_no), (144, 2, 3));
            assert_eq!(back.random_bytes, vec![1; 31]);
            assert!(
                matches!(back.message,enums::DecryptedMessage::Service(s)if matches!(&s.action,enums::DecryptedMessageAction::NotifyLayer(a)if a.layer==144))
            );
        }
        "test_a_vector_of_objects_round_trips" => {
            let mut m = message();
            m.entities = Some(vec![
                types::MessageEntityBold {
                    offset: 0,
                    length: 2,
                }
                .into(),
            ]);
            let m = back(m);
            assert_eq!(m.entities.as_ref().unwrap().len(), 1);
            assert!(matches!(&m.entities.unwrap()[0],enums::MessageEntity::Bold(e)if e.offset==0));
        }
        "test_the_layer_8_service_shape_is_accepted" => {
            let m: enums::DecryptedMessage = types::DecryptedMessageServiceLegacyaa48327d {
                random_id: 1,
                random_bytes: vec![2; 15],
                action: enums::DecryptedMessageAction::Noop,
            }
            .into();
            let data = m.to_bytes();
            wire::validate(&data, "DecryptedMessage").unwrap();
            assert!(
                matches!(enums::DecryptedMessage::from_bytes(&data).unwrap(),enums::DecryptedMessage::ServiceLegacyaa48327d(s)if s.random_id==1)
            );
        }
        "test_an_unknown_constructor_is_reported_not_guessed" => {
            assert!(wire::validate(&[0xde, 0xad, 0xbe, 0xef], "DecryptedMessage").is_err());
            super::wire_properties::check();
        }
        "test_a_nested_layer_is_refused_at_the_declared_message_type_boundary" => {
            let service: enums::DecryptedMessage = types::DecryptedMessageService {
                random_id: 1,
                action: enums::DecryptedMessageAction::Noop,
            }
            .into();
            let wrapper = types::DecryptedMessageLayer {
                random_bytes: vec![0; 15],
                layer: 144,
                in_seq_no: 0,
                out_seq_no: 0,
                message: service.clone(),
            };
            let e: enums::DecryptedMessageLayer = wrapper.into();
            let prefix = e.to_bytes();
            let prefix = &prefix[..prefix.len() - service.to_bytes().len()];
            let mut malformed = Vec::new();
            for _ in 0..64 {
                malformed.extend_from_slice(prefix);
            }
            malformed.extend_from_slice(&service.to_bytes());
            assert!(wire::validate(&malformed, "DecryptedMessageLayer").is_err());
            let mut wrong = enums::DecryptedMessageLayer::from(types::DecryptedMessageLayer {
                random_bytes: vec![0; 31],
                layer: 144,
                in_seq_no: 0,
                out_seq_no: 1,
                message: message().into(),
            })
            .to_bytes();
            wrong[48..52].copy_from_slice(&enums::DecryptedMessageAction::Noop.to_bytes());
            assert!(wire::validate(&wrong, "DecryptedMessageLayer").is_err());
        }
        "test_untyped_root_parsing_still_enforces_the_depth_bound" => {
            let empty = enums::DecryptedMessageMedia::Empty.to_bytes();
            assert!(
                wire::validate_at_depth(&empty, "DecryptedMessageMedia", wire::MAX_NESTING)
                    .is_err()
            );
            wire::validate_at_depth(&empty, "DecryptedMessageMedia", wire::MAX_NESTING - 1)
                .unwrap();
        }
        _ => panic!("unknown schema scenario {case}"),
    }
}
