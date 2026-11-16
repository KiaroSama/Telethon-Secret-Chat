#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use grammers_tl_types::{enums::MessageEntity as ApiEntity, types as api};
use kiaro_secret_chat::{
    Identifiable, Serializable,
    entities::{self, EntityInput},
    enums,
    manager::{Event, MessageReceived},
    types,
};
use std::sync::{Arc, Mutex};
#[test]
fn blockquote_gets_end_to_end_constructor() {
    for collapsed in [false, true] {
        let api: ApiEntity = api::MessageEntityBlockquote {
            collapsed,
            offset: 3,
            length: 4,
        }
        .into();
        let converted = entities::convert(vec![api.clone()], 144);
        let expected = [
            0x020df5d0_u32.to_le_bytes(),
            3_i32.to_le_bytes(),
            4_i32.to_le_bytes(),
        ]
        .concat();
        assert_eq!(converted[0].to_bytes(), expected);
        assert_ne!(converted[0].to_bytes()[..4], api.to_bytes()[..4]);
        assert_eq!((converted[0].offset(), converted[0].length()), (3, 4));
    }

    let e = entities::convert(
        vec![
            api::MessageEntityBlockquote {
                collapsed: false,
                offset: 0,
                length: 3,
            }
            .into(),
        ],
        144,
    );
    assert_eq!(e.len(), 1);
    assert!(matches!(e[0], enums::MessageEntity::Blockquote(_)));
    assert_eq!(
        u32::from_le_bytes(e[0].to_bytes()[..4].try_into().unwrap()),
        0x020df5d0
    );
}
#[test]
fn missing_entity_is_dropped() {
    for entity in unsupported_entities() {
        assert!(entities::convert(vec![entity.clone()], 144).is_empty());
        assert!(entities::convert_inputs(vec![EntityInput::Api(entity)], 144).is_empty());
    }

    let e = entities::convert(
        vec![
            api::MessageEntityMentionName {
                offset: 0,
                length: 3,
                user_id: 1,
            }
            .into(),
        ],
        144,
    );
    assert!(e.is_empty());
}
#[test]
fn newer_entity_is_dropped_until_supported() {
    let mut supported = supported_entities(3, 4);
    supported.push((
        api::MessageEntityBlockquote {
            collapsed: true,
            offset: 3,
            length: 4,
        }
        .into(),
        101,
    ));
    for (source, minimum) in supported {
        let secret = entities::convert(vec![source.clone()], 144).pop().unwrap();
        for layer in [46, 73, 100, 101, 143, 144, 201] {
            let api_result = entities::convert(vec![source.clone()], layer);
            let secret_result =
                entities::convert_inputs(vec![EntityInput::Secret(secret.clone())], layer);
            assert_eq!(api_result.len(), usize::from(layer >= minimum));
            assert_eq!(secret_result.len(), usize::from(layer >= minimum));
            if layer >= minimum {
                assert_eq!(api_result[0].to_bytes(), secret_result[0].to_bytes());
            }
        }
    }

    let e = api::MessageEntityUnderline {
        offset: 0,
        length: 3,
    };
    assert!(entities::convert(vec![e.clone().into()], 73).is_empty());
    let v = entities::convert(vec![e.into()], 144);
    assert_eq!(
        u32::from_le_bytes(v[0].to_bytes()[..4].try_into().unwrap()),
        0x9c4e7e8b
    );
}
#[test]
fn extra_fields_are_carried() {
    // API and end-to-end entities have independent generated encoders. These
    // shared constructors must retain their exact signed TL fields and extras.
    for (offset, length) in [(0, 0), (3, 4), (i32::MAX - 1, 1), (i32::MAX, i32::MAX)] {
        for (source, _) in supported_entities(offset, length) {
            let result = entities::convert(vec![source.clone()], 144);
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].to_bytes(), source.to_bytes());
            assert_eq!((result[0].offset(), result[0].length()), (offset, length));
            kiaro_secret_chat::wire::validate(&result[0].to_bytes(), "MessageEntity").unwrap();
        }
    }

    let v = entities::convert(
        vec![
            api::MessageEntityTextUrl {
                offset: 1,
                length: 2,
                url: "https://example.org".into(),
            }
            .into(),
        ],
        73,
    );
    let enums::MessageEntity::TextUrl(e) = &v[0] else {
        panic!("wrong entity")
    };
    assert_eq!(
        (e.offset, e.length, e.url.as_str()),
        (1, 2, "https://example.org")
    );
}
#[test]
fn secret_entity_passes_through() {
    let sources = supported_entities(3, 4);
    let mut mixed = Vec::new();
    let mut expected_mixed = Vec::new();
    for (api, _) in sources {
        let secret = entities::convert(vec![api.clone()], 144).pop().unwrap();
        expected_mixed.extend([secret.to_bytes(), secret.to_bytes()]);
        mixed.extend([EntityInput::Api(api), EntityInput::Secret(secret)]);
    }
    assert_eq!(
        entities::convert_inputs(mixed, 144)
            .iter()
            .map(Serializable::to_bytes)
            .collect::<Vec<_>>(),
        expected_mixed
    );

    let e: enums::MessageEntity = types::MessageEntityBold {
        offset: 0,
        length: 1,
    }
    .into();
    let expected = e.to_bytes();
    let result = entities::convert_inputs(vec![EntityInput::Secret(e)], 73);
    assert_eq!(result[0].to_bytes(), expected);
    assert_eq!(
        types::MessageEntityBold::CONSTRUCTOR_ID,
        u32::from_le_bytes(expected[..4].try_into().unwrap())
    );
}
#[tokio::test]
async fn formatted_text_uses_default_and_arrives_mapped() {
    let (a, b, world, id) = support::established().await;
    let got = Arc::new(Mutex::new(Vec::<MessageReceived>::new()));
    let sink = got.clone();
    b.on("MessageReceived", "entity_default", move |e| {
        if let Event::MessageReceived(m) = e {
            sink.lock().unwrap().push(*m);
        }
        Ok(())
    })
    .unwrap();
    a.send_message(id, "**bold** text", None, None)
        .await
        .unwrap();
    support::pump(&a, &b, &world).await;
    for input in [
        "🦀 **bold** and *italic*",
        "**e\u{301} 😀**",
        "🦀 **unterminated",
        "🦀 [site](https://example.org) and `code`",
    ] {
        a.send_message(id, input, None, None).await.unwrap();
    }
    a.send_message(id, "**literal 🦀**", Some(vec![]), None)
        .await
        .unwrap();
    a.send_message(
        id,
        "😀ABC",
        Some(vec![
            api::MessageEntityBold {
                offset: 2,
                length: 3,
            }
            .into(),
        ]),
        None,
    )
    .await
    .unwrap();
    a.send_message(
        id,
        "profile",
        Some(vec![
            api::MessageEntityMentionName {
                offset: 0,
                length: 7,
                user_id: 123,
            }
            .into(),
        ]),
        None,
    )
    .await
    .unwrap();
    support::pump(&a, &b, &world).await;
    let messages = got.lock().unwrap().clone();
    let m = &messages[0];
    assert_eq!(m.text, "bold text");
    assert!(matches!(
        m.entities.as_ref().unwrap()[0],
        enums::MessageEntity::Bold(_)
    ));
    assert_eq!(messages.len(), 8);
    assert_eq!(messages[1].text, "🦀 bold and italic");
    let formatted = messages[1].entities.as_ref().unwrap();
    assert!(
        formatted
            .iter()
            .any(|e| matches!(e,enums::MessageEntity::Bold(e)if(e.offset,e.length)==(3,4)))
    );
    assert!(
        formatted
            .iter()
            .any(|e| matches!(e,enums::MessageEntity::Italic(e)if(e.offset,e.length)==(12,6)))
    );
    for message in &messages {
        let units = message.text.encode_utf16().collect::<Vec<_>>();
        for entity in message.entities.iter().flatten() {
            let offset = usize::try_from(entity.offset()).unwrap();
            let length = usize::try_from(entity.length()).unwrap();
            assert!(offset.checked_add(length).unwrap() <= units.len());
            assert!(String::from_utf16(&units[offset..offset + length]).is_ok());
        }
    }
    assert_eq!(messages[2].text, "e\u{301} 😀");
    assert!(
        matches!(messages[2].entities.as_ref().unwrap()[0],enums::MessageEntity::Bold(ref e)if(e.offset,e.length)==(0,5))
    );
    assert_eq!(messages[3].text, "🦀 **unterminated");
    assert!(messages[3].entities.is_none());
    assert_eq!(messages[4].text, "🦀 site and code");
    assert!(messages[4].entities.as_ref().unwrap().iter().any(|e|matches!(e,enums::MessageEntity::TextUrl(e)if(e.offset,e.length,e.url.as_str())==(3,4,"https://example.org"))));
    assert_eq!(messages[5].text, "**literal 🦀**");
    assert!(messages[5].entities.is_none());
    assert_eq!(messages[7].text, "profile");
    assert!(
        messages[7].entities.is_none(),
        "unrepresentable formatting is dropped while the text still arrives"
    );
    assert_eq!(messages[6].text, "😀ABC");
    assert!(
        matches!(messages[6].entities.as_ref().unwrap()[0],enums::MessageEntity::Bold(ref e)if(e.offset,e.length)==(2,3))
    );
}

fn supported_entities(offset: i32, length: i32) -> Vec<(ApiEntity, i32)> {
    macro_rules! basic {
        ($typ:ident) => {
            (api::$typ { offset, length }.into(), 46)
        };
    }
    vec![
        basic!(MessageEntityUnknown),
        basic!(MessageEntityMention),
        basic!(MessageEntityHashtag),
        basic!(MessageEntityBotCommand),
        basic!(MessageEntityUrl),
        basic!(MessageEntityEmail),
        basic!(MessageEntityBold),
        basic!(MessageEntityItalic),
        basic!(MessageEntityCode),
        (
            api::MessageEntityPre {
                offset,
                length,
                language: "rust🦀".into(),
            }
            .into(),
            46,
        ),
        (
            api::MessageEntityTextUrl {
                offset,
                length,
                url: "https://example.org/🦀?q=1&x=2".into(),
            }
            .into(),
            46,
        ),
        (api::MessageEntityUnderline { offset, length }.into(), 101),
        (api::MessageEntityStrike { offset, length }.into(), 101),
        (api::MessageEntitySpoiler { offset, length }.into(), 144),
        (
            api::MessageEntityCustomEmoji {
                offset,
                length,
                document_id: i64::MIN,
            }
            .into(),
            144,
        ),
    ]
}
fn unsupported_entities() -> Vec<ApiEntity> {
    let (offset, length) = (3, 4);
    vec![
        api::MessageEntityMentionName {
            offset,
            length,
            user_id: 1,
        }
        .into(),
        api::InputMessageEntityMentionName {
            offset,
            length,
            user_id: grammers_tl_types::enums::InputUser::UserSelf,
        }
        .into(),
        api::MessageEntityPhone { offset, length }.into(),
        api::MessageEntityCashtag { offset, length }.into(),
        api::MessageEntityBankCard { offset, length }.into(),
        api::MessageEntityFormattedDate {
            relative: true,
            short_time: false,
            long_time: false,
            short_date: true,
            long_date: false,
            day_of_week: true,
            offset,
            length,
            date: 123,
        }
        .into(),
        api::MessageEntityDiffInsert { offset, length }.into(),
        api::MessageEntityDiffDelete { offset, length }.into(),
        api::MessageEntityDiffReplace {
            offset,
            length,
            old_text: "before".into(),
        }
        .into(),
        api::MessageEntityTonAddress { offset, length }.into(),
    ]
}
