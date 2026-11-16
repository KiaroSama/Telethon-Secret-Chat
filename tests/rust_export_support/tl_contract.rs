//! The public typed TL bridge must preserve every Serde value shape it accepts.
#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::tl_value::{request_args, to_value};
use serde::{Serialize, Serializer, de::DeserializeOwned, ser::SerializeMap};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Serialize)]
struct Record {
    flag: bool,
    value: Option<i64>,
}
#[derive(Serialize)]
struct Unit;
#[derive(Serialize)]
struct Newtype(i32);
#[derive(Serialize)]
struct Pair(i32, bool);
#[derive(Serialize)]
enum Shape {
    Unit,
    Newtype(Record),
    Tuple(i32, bool),
    Named { flag: bool },
}
struct Bytes<'a>(&'a [u8]);
impl Serialize for Bytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}
struct MissingMapKey;
impl Serialize for MissingMapKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_value("public value")?;
        map.end()
    }
}
fn official_request<T: DeserializeOwned + grammers_tl_types::Serializable>(
    input: Value,
    expected: T,
) {
    let actual: T = serde_json::from_value(request_args(&input)).unwrap();
    assert_eq!(actual.to_bytes(), expected.to_bytes());
}

pub fn assert_native_tl_bridge_shapes() {
    use grammers_tl_types::{enums as api, types as raw};
    official_request(json!({"_":"InputPeerSelf"}), api::InputPeer::PeerSelf);
    official_request(json!({"_":"InputUserSelf"}), api::InputUser::UserSelf);
    official_request::<api::InputUser>(
        json!({"_":"InputUser","user_id":1,"access_hash":2}),
        raw::InputUser {
            user_id: 1,
            access_hash: 2,
        }
        .into(),
    );
    official_request::<api::InputChannel>(
        json!({"_":"InputChannel","channel_id":3,"access_hash":5}),
        raw::InputChannel {
            channel_id: 3,
            access_hash: 5,
        }
        .into(),
    );
    official_request::<api::InputMessage>(
        json!({"_":"InputMessageID","id":7}),
        raw::InputMessageId { id: 7 }.into(),
    );
    official_request::<api::InputFileLocation>(
        json!({"_":"InputDocumentFileLocation","id":11,"access_hash":13,"file_reference":[0,127,255],"thumb_size":"m"}),
        raw::InputDocumentFileLocation {
            id: 11,
            access_hash: 13,
            file_reference: vec![0, 127, 255],
            thumb_size: "m".into(),
        }
        .into(),
    );
    official_request::<api::InputFileLocation>(
        json!({"_":"InputPhotoFileLocation","id":17,"access_hash":19,"file_reference":[1,2],"thumb_size":"y"}),
        raw::InputPhotoFileLocation {
            id: 17,
            access_hash: 19,
            file_reference: vec![1, 2],
            thumb_size: "y".into(),
        }
        .into(),
    );
    official_request(
        json!({"_":"InputTakeoutFileLocation"}),
        api::InputFileLocation::InputTakeoutFileLocation,
    );
    official_request(
        json!({"_":"InputMessagesFilterEmpty"}),
        api::MessagesFilter::InputMessagesFilterEmpty,
    );
    official_request(
        json!({"_":"InputMessagesFilterPhotos"}),
        api::MessagesFilter::InputMessagesFilterPhotos,
    );
    official_request(
        json!({"_":"InputMessagesFilterVideo"}),
        api::MessagesFilter::InputMessagesFilterVideo,
    );
    assert!(
        serde_json::from_value::<api::InputFileLocation>(request_args(
            &json!({"_":"InputDocumentFileLocation"})
        ))
        .is_err()
    );
    assert_eq!(to_value(&-7_i8).unwrap(), json!(-7));
    assert_eq!(to_value(&-71_i16).unwrap(), json!(-71));
    assert_eq!(to_value(&-711_i32).unwrap(), json!(-711));
    assert_eq!(to_value(&i64::MIN).unwrap(), json!(i64::MIN));
    assert_eq!(to_value(&7_u8).unwrap(), json!(7));
    assert_eq!(to_value(&71_u16).unwrap(), json!(71));
    assert_eq!(to_value(&711_u32).unwrap(), json!(711));
    assert_eq!(to_value(&u64::MAX).unwrap(), json!(u64::MAX));
    assert_eq!(to_value(&0.5_f32).unwrap(), json!(0.5));
    assert_eq!(to_value(&0.25_f64).unwrap(), json!(0.25));
    assert_eq!(to_value(&'Ω').unwrap(), json!("Ω"));
    assert_eq!(
        to_value(&Bytes(&[0, 127, 255])).unwrap(),
        json!([0, 127, 255])
    );
    assert_eq!(to_value(&None::<i64>).unwrap(), json!(null));
    assert_eq!(to_value(&Some(-17_i64)).unwrap(), json!(-17));
    assert_eq!(to_value(&()).unwrap(), json!(null));
    assert_eq!(to_value(&Unit).unwrap(), json!({"_":"Unit"}));
    assert_eq!(to_value(&Shape::Unit).unwrap(), json!({"_":"Unit"}));
    assert_eq!(to_value(&Newtype(19)).unwrap(), json!(19));
    assert_eq!(
        to_value(&Shape::Newtype(Record {
            flag: true,
            value: None
        }))
        .unwrap(),
        json!({"_":"Record","flag":true,"value":null})
    );
    assert_eq!(to_value(&(23_i32, false)).unwrap(), json!([23, false]));
    assert_eq!(to_value(&Pair(29, true)).unwrap(), json!([29, true]));
    assert_eq!(
        to_value(&Shape::Tuple(31, false)).unwrap(),
        json!([31, false])
    );
    assert_eq!(
        to_value(&Shape::Named { flag: true }).unwrap(),
        json!({"_":"Named","flag":true})
    );
    assert_eq!(
        to_value(&BTreeMap::from([("key", vec![1_i32, 2])])).unwrap(),
        json!({"key":[1,2]})
    );
    assert_eq!(
        to_value(&BTreeMap::from([(7_i32, "public")])).unwrap(),
        json!({"7":"public"})
    );
    assert_eq!(
        to_value(&MissingMapKey).unwrap_err().to_string(),
        "Invalid TL map"
    );

    // These are the public SDK's enum spelling rules, not secret-schema tags.
    assert_eq!(
        request_args(&json!({"_":"InputUserSelf"})),
        json!("UserSelf")
    );
    assert_eq!(
        request_args(&json!({"_":"InputUser","user_id":1,"access_hash":2})),
        json!({"User":{"user_id":1,"access_hash":2}})
    );
    assert_eq!(
        request_args(&json!({"_":"InputChannel","channel_id":3,"access_hash":5})),
        json!({"Channel":{"channel_id":3,"access_hash":5}})
    );
    assert_eq!(
        request_args(&json!({"_":"InputMessageID","id":7})),
        json!({"Id":{"id":7}})
    );
    for name in [
        "InputDocumentFileLocation",
        "InputPhotoFileLocation",
        "InputTakeoutFileLocation",
        "InputMessagesFilterEmpty",
    ] {
        assert_eq!(request_args(&json!({"_":name})), json!(name));
    }
    assert_eq!(
        request_args(&json!([{"_":"InputPeerSelf"}, 11])),
        json!(["PeerSelf", 11])
    );
    assert_eq!(
        request_args(&json!({"_":"NonRequestData","nested":{"value":13}})),
        json!({"nested":{"value":13}})
    );
}
