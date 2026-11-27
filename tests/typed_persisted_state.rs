#![forbid(unsafe_code)]

use kiaro_secret_chat::protocol::typed_state::{
    PersistedGapState, PersistedRekeyExchange, TYPED_PROTOCOL_FORMAT_V2, decode_v2_json,
    migrate_json_v1_to_v2,
};
use kiaro_secret_chat::protocol::{ChatRole, SecretChat, encrypt_frame, wrap_frame};
use serde_json::{Value, json};

const KEY: [u8; 256] = [0xac; 256];

fn chat() -> SecretChat {
    let mut chat = SecretChat::new(7, 8, 9, "synthetic-typed-state", ChatRole::Acceptor);
    chat.adopt_key(&KEY).unwrap();
    chat
}

fn flat() -> Value {
    let snapshot = serde_json::to_value(chat().export_typed_runtime().unwrap()).unwrap();
    let mut flat = snapshot.as_object().unwrap().clone();
    let rekey = flat.remove("rekey").unwrap();
    flat.extend(
        rekey
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, _)| *key != "phase")
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    flat.remove("gap");
    flat.extend(
        json!({
            "format_version": 1, "gap_requested": false, "gap_end": null,
            "resend_due": null, "resend_request": null, "exchange": null,
            "exchange_role": null, "secret_key": null, "pending_key": null
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    Value::Object(flat)
}

fn round_trip(source: &SecretChat) {
    let mut expected = serde_json::to_value(source.export_typed_runtime().unwrap()).unwrap();
    let decoded = decode_v2_json(&expected.to_string()).unwrap();
    let mut restored = chat();
    restored.import_typed_runtime(&decoded).unwrap();
    let mut actual = serde_json::to_value(restored.export_typed_runtime().unwrap()).unwrap();
    // Export measures elapsed wall-clock time independently on each side.
    expected
        .as_object_mut()
        .unwrap()
        .remove("rekey_age_seconds");
    actual.as_object_mut().unwrap().remove("rekey_age_seconds");
    assert_eq!(actual, expected);
}

#[test]
fn idle_and_buffered_gap_round_trip() {
    let mut receiver = chat();
    round_trip(&receiver);
    let body = wrap_frame(&json!({"message": "second", "random_id": 42}), 73, 0, 3).unwrap();
    let frame = encrypt_frame(&KEY, &body, receiver.in_x()).unwrap();
    assert!(
        receiver
            .receive_encrypted_frames(&frame)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        receiver.export_typed_runtime().unwrap().gap,
        PersistedGapState::Active { .. }
    ));
    round_trip(&receiver);
}

#[test]
fn active_rekey_migrates_and_round_trips_with_redacted_debug() {
    for role in ["Requested", "Accepted"] {
        let mut v1 = flat();
        let fingerprint = kiaro_secret_chat::protocol::key_fingerprint(&KEY).unwrap();
        v1["key_lifecycle"] = json!({"Rekeying": {"fingerprint": fingerprint, "exchange_id": 42}});
        v1["exchange_role"] = json!(role);
        v1["exchange"] = json!(42);
        let key_field = if role == "Requested" {
            "secret_key"
        } else {
            "pending_key"
        };
        assert!(migrate_json_v1_to_v2(&v1.to_string()).is_err());
        v1[key_field] = json!(vec![219u8; 256]);
        let decoded = decode_v2_json(&migrate_json_v1_to_v2(&v1.to_string()).unwrap()).unwrap();
        assert!(matches!(
            (&decoded.rekey, role),
            (PersistedRekeyExchange::Requested { .. }, "Requested")
                | (PersistedRekeyExchange::Accepted { .. }, "Accepted")
        ));
        for debug in [format!("{decoded:?}"), format!("{:?}", decoded.rekey)] {
            assert!(!debug.contains("172, 172") && !debug.contains("219, 219"));
            assert!(!debug.contains("acacac") && !debug.contains("dbdbdb"));
        }
        let mut restored = chat();
        restored.import_typed_runtime(&decoded).unwrap();
        round_trip(&restored);
    }
}

#[test]
fn rejects_wrong_versions_unknown_fields_and_duplicate_keys() {
    let mut v1 = flat();
    let v2 = migrate_json_v1_to_v2(&v1.to_string()).unwrap();
    assert_eq!(
        decode_v2_json(&v2).unwrap().format_version,
        TYPED_PROTOCOL_FORMAT_V2
    );
    v1["gap_end"] = json!(4);
    assert!(migrate_json_v1_to_v2(&v1.to_string()).is_err());
    v1["gap_end"] = Value::Null;
    v1["format_version"] = json!(2);
    assert!(migrate_json_v1_to_v2(&v1.to_string()).is_err());
    v1["format_version"] = json!(1);
    v1["unknown"] = Value::Null;
    assert!(migrate_json_v1_to_v2(&v1.to_string()).is_err());
    for (from, to) in [
        ("\"format_version\":2", "\"format_version\":1"),
        (
            "\"format_version\":2",
            "\"format_version\":2,\"format_version\":2",
        ),
        ("\"phase\":\"Idle\"", "\"phase\":\"Idle\",\"unknown\":null"),
        (
            "\"pending_frame_keys\":{}",
            "\"pending_frame_keys\":{\"1\":0,\"1\":0}",
        ),
    ] {
        assert!(v2.contains(from));
        assert!(decode_v2_json(&v2.replace(from, to)).is_err());
    }
}
