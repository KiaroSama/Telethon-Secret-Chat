//! Known constructor IDs remain invalid in another TL result family.
#![allow(clippy::unwrap_used)]
mod support_protocol;
use kiaro_secret_chat::{Serializable, enums, error::Error, framing, types};
use support_protocol::*;
fn document() -> types::DecryptedMessageMediaDocument {
    types::DecryptedMessageMediaDocument {
        thumb: vec![],
        thumb_w: 0,
        thumb_h: 0,
        mime_type: "application/octet-stream".into(),
        size: 16,
        key: vec![1; 32],
        iv: vec![2; 32],
        attributes: vec![],
        caption: String::new(),
    }
}
fn replace_one(bytes: &mut Vec<u8>, before: &[u8], after: &[u8]) {
    let positions = bytes
        .windows(before.len())
        .enumerate()
        .filter_map(|(i, b)| (b == before).then_some(i))
        .collect::<Vec<_>>();
    assert_eq!(positions.len(), 1);
    let i = positions[0];
    bytes.splice(i..i + before.len(), after.iter().copied());
}
fn wrong_family(kind: &str) {
    let mut wrapper = wrap(1, 0, 144);
    let enums::DecryptedMessage::Message(ref mut message) = wrapper.message else {
        unreachable!()
    };
    let mut original = enums::DecryptedMessageMedia::Empty.to_bytes();
    let mut wrong = enums::DecryptedMessageAction::Noop.to_bytes();
    match kind {
        "media" => message.media = Some(enums::DecryptedMessageMedia::Empty),
        "entities" => {
            let entity: enums::MessageEntity = types::MessageEntityBold {
                offset: 0,
                length: 1,
            }
            .into();
            original = entity.to_bytes();
            message.entities = Some(vec![entity]);
        }
        "action" => {
            original = enums::DecryptedMessageAction::Noop.to_bytes();
            wrong = enums::DecryptedMessageMedia::Empty.to_bytes();
            wrapper.message = types::DecryptedMessageService {
                random_id: 1,
                action: enums::DecryptedMessageAction::Noop,
            }
            .into();
        }
        "attribute" => {
            let mut media = document();
            let attribute: enums::DocumentAttribute = types::DocumentAttributeFilename {
                file_name: "x".into(),
            }
            .into();
            original = attribute.to_bytes();
            media.attributes.push(attribute);
            message.media = Some(media.into());
        }
        "stickerset" => {
            let mut media = document();
            original = enums::InputStickerSet::Empty.to_bytes();
            media.attributes.push(
                types::DocumentAttributeSticker {
                    alt: String::new(),
                    stickerset: enums::InputStickerSet::Empty,
                }
                .into(),
            );
            message.media = Some(media.into());
        }
        _ => unreachable!(),
    }
    let mut body = enums::DecryptedMessageLayer::from(wrapper).to_bytes();
    assert!(framing::unwrap(&body).is_ok());
    replace_one(&mut body, &original, &wrong);
    assert!(matches!(
        framing::unwrap(&body),
        Err(Error::InnerMessageRejected)
    ));
}
#[test]
fn action_is_not_media() {
    wrong_family("media");
}
#[test]
fn action_is_not_entity() {
    wrong_family("entities");
}
#[test]
fn media_is_not_action() {
    wrong_family("action");
}
#[test]
fn action_is_not_document_attribute() {
    wrong_family("attribute");
}
#[test]
fn action_is_not_sticker_set() {
    wrong_family("stickerset");
}
