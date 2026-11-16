#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{Deserializable, Serializable, enums, wire};
pub fn check(id: u64) {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/rust-schema-oracle.json")).unwrap();
    let r = data["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"].as_u64() == Some(id))
        .unwrap();
    let bytes = hex::decode(r["hex"].as_str().unwrap()).unwrap();
    let family = r["family"].as_str().unwrap();
    wire::validate(&bytes, family).unwrap();
    let encoded = match family {
        "DecryptedMessage" => enums::DecryptedMessage::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageAction" => enums::DecryptedMessageAction::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageLayer" => enums::DecryptedMessageLayer::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageMedia" => enums::DecryptedMessageMedia::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DocumentAttribute" => enums::DocumentAttribute::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "FileLocation" => enums::FileLocation::from_bytes(&bytes).unwrap().to_bytes(),
        "GroupCallMessage" => enums::GroupCallMessage::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "InputStickerSet" => enums::InputStickerSet::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "JSONObjectValue" => enums::JsonobjectValue::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "JSONValue" => enums::Jsonvalue::from_bytes(&bytes).unwrap().to_bytes(),
        "MessageEntity" => enums::MessageEntity::from_bytes(&bytes).unwrap().to_bytes(),
        "PhotoSize" => enums::PhotoSize::from_bytes(&bytes).unwrap().to_bytes(),
        "SendMessageAction" => enums::SendMessageAction::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "TextWithEntities" => enums::TextWithEntities::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        _ => panic!("Unknown oracle family"),
    };
    assert_eq!(encoded, bytes);
}

pub fn check_all() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/rust-schema-oracle.json")).unwrap();
    let records = value["records"].as_array().unwrap();
    assert_eq!(records.len(), 90);
    for record in records {
        check(record["id"].as_u64().unwrap());
    }
}
