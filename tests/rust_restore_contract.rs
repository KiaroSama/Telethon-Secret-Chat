#![allow(clippy::unwrap_used)]
#[path = "support_protocol/error_context.rs"]
mod error_context;
mod support_protocol;
use kiaro_secret_chat::{
    chat::ChatState,
    protocol::Protocol,
    storage::{FileStorage, State, Storage},
};
use support_protocol::*;
#[path = "support_manager/mod.rs"]
mod manager_support;
#[test]
fn closed_legacy_record_scrubs_keys_and_queues_without_revival() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    let mut c = chat(true, 1);
    c.previous_key = Some(key());
    c.gaps.insert(3, envelope(3, 0, 144, true));
    state.chats.insert(7, c);
    store.commit(&state).unwrap();
    drop(store);
    let mut old: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    old["chats"]["7"]["state"] = "closed".into();
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let p = Protocol::open(FileStorage::open(&path).unwrap(), "11".repeat(32)).unwrap();
    let closed = p.chat_record(7).unwrap();
    assert_eq!(closed.state, ChatState::Closed);
    assert!(
        closed.key.is_none()
            && closed.previous_key.is_none()
            && closed.gaps.is_empty()
            && closed.outbox.is_empty()
    );
    drop(p);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved["chats"]["7"]["key"].is_null() && saved["chats"]["7"]["previous_key"].is_null());
    // A protected custody record is never eligible for automatic key erasure.
    old["mode"] = "protected".into();
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let store = FileStorage::open(&path).unwrap();
    assert!(store.load().is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[test]
fn corrupt_record_errors_name_only_the_safe_chat_identity() {
    error_context::conversation_context_keeps_useful_identity_without_exposing_metadata();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    store.commit(&state).unwrap();
    let mut invalid: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    invalid["chats"]["7"]["key"] = "00".repeat(255).into();
    let bytes = serde_json::to_vec(&invalid).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let error = match store.load() {
        Err(error) => error,
        Ok(_) => panic!("The corrupt key was accepted"),
    };
    let text = format!("{error} {error:?}");
    assert!(
        text.contains('7'),
        "restoration refusals must identify the affected conversation"
    );
    assert!(!text.contains(&"00".repeat(16)));
    assert!(text.len() < 512);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
fn refused_field(field: &str, value: serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    store.commit(&state).unwrap();
    drop(store);
    let mut invalid: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    invalid["chats"]["7"]
        .as_object_mut()
        .unwrap()
        .insert(field.into(), value);
    let bytes = serde_json::to_vec(&invalid).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let (transport, _) = manager_support::Fake::pair();
    let world = std::sync::Arc::clone(&transport.world);
    let result = kiaro_secret_chat::manager::SecretChatManager::new(
        transport,
        FileStorage::open(&path).unwrap(),
        "11".repeat(32),
        Default::default(),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("A corrupt record installed a manager"),
    };
    let text = format!("{error} {error:?}");
    assert!(text.contains('7'));
    assert!(!text.contains(&"07".repeat(16)));
    assert!(text.len() < 512);
    let world = world.lock().unwrap();
    assert!(world.sent.is_empty() && world.requests.is_empty() && world.discards.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
fn corrupt_service_mailbox_refuses_restoration() {
    use kiaro_secret_chat::{Serializable, chat::ServiceDelivery, enums, types};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service-state.json");
    let mut c = chat(true, 0);
    c.in_seq_no = 1;
    let body = kiaro_secret_chat::framing::wrap(
        types::DecryptedMessageService {
            random_id: 1,
            action: enums::DecryptedMessageAction::Noop,
        }
        .into(),
        144,
        1,
        0,
    )
    .unwrap()
    .to_bytes();
    c.service_deliveries.insert(
        0,
        ServiceDelivery {
            envelope: kiaro_secret_chat::chat::Envelope {
                body: body.clone(),
                file: None,
                date: 1,
            },
            applied: None,
        },
    );
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, c);
    let mut store = FileStorage::open(&path).unwrap();
    store.commit(&state).unwrap();
    assert!(store.load().unwrap().is_some());
    drop(store);
    let base = serde_json::to_value(&state).unwrap();
    for change in 0..14 {
        let mut invalid = base.clone();
        let c = &mut invalid["chats"]["7"];
        match change {
            0 => {
                let row = c["service_deliveries"]
                    .as_object_mut()
                    .unwrap()
                    .remove("0")
                    .unwrap();
                c["service_deliveries"]["-1"] = row;
            }
            1 => {
                let row = c["service_deliveries"]
                    .as_object_mut()
                    .unwrap()
                    .remove("0")
                    .unwrap();
                c["service_deliveries"]["1"] = row;
                c["in_seq_no"] = 2.into();
            }
            2 => c["service_deliveries"]["0"]["envelope"]["body"] = serde_json::json!([1, 2, 3]),
            3 => {
                c["service_deliveries"]["0"]["envelope"]["body"] =
                    serde_json::to_value(envelope(0, 0, 144, true).body).unwrap()
            }
            4 => c["in_seq_no"] = 0.into(),
            5 => {
                let mut body = body.clone();
                body[44..48].copy_from_slice(&1i32.to_le_bytes());
                c["service_deliveries"]["0"]["envelope"]["body"] =
                    serde_json::to_value(body).unwrap();
            }
            6 => c["service_deliveries"]["0"]["envelope"]["file"] = serde_json::json!([1, 2, 3]),
            7 => c["deliveries"]["0"] = serde_json::to_value(envelope(0, 0, 144, true)).unwrap(),
            8 => {
                let mut body = body.clone();
                body[40..44].copy_from_slice(&0i32.to_le_bytes());
                c["service_deliveries"]["0"]["envelope"]["body"] =
                    serde_json::to_value(body).unwrap();
            }
            9 => c["gaps"]["1"] = serde_json::to_value(envelope(1, 0, 144, true)).unwrap(),
            10 => c["gaps"]["0"] = serde_json::to_value(envelope(0, 0, 144, true)).unwrap(),
            11 => c["deliveries"]["1"] = serde_json::to_value(envelope(1, 0, 144, true)).unwrap(),
            12 => {
                c["service_deliveries"] = serde_json::json!({});
                let mut ordinary = envelope(0, 0, 144, true);
                ordinary.body[44..48].copy_from_slice(&1i32.to_le_bytes());
                c["deliveries"]["0"] = serde_json::to_value(ordinary).unwrap();
            }
            13 => {
                c["service_deliveries"] = serde_json::json!({});
                c["deliveries"]["0"] = serde_json::to_value(kiaro_secret_chat::chat::Envelope {
                    body: body.clone(),
                    file: None,
                    date: 1,
                })
                .unwrap();
            }
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (transport, _) = manager_support::Fake::pair();
        let w = transport.world.clone();
        let result = kiaro_secret_chat::manager::SecretChatManager::new(
            transport,
            FileStorage::open(&path).unwrap(),
            "11".repeat(32),
            Default::default(),
        );
        assert!(
            result.is_err(),
            "corrupt persisted service variant {change} installed a manager"
        );
        assert!(w.lock().unwrap().sent.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
macro_rules! runtime_field {
    ($name:ident,$field:expr,$value:expr) => {
        #[test]
        fn $name() {
            refused_field($field, $value);
        }
    };
}
runtime_field!(
    runtime_outbound_string_refused,
    "is_outbound",
    serde_json::json!("false")
);
runtime_field!(runtime_state_list_refused, "state", serde_json::json!([]));
runtime_field!(
    runtime_access_null_refused,
    "access_hash",
    serde_json::Value::Null
);
runtime_field!(
    runtime_peer_string_refused,
    "peer_user_id",
    serde_json::json!("9")
);
runtime_field!(runtime_negative_ttl_refused, "ttl", serde_json::json!(-1));
runtime_field!(runtime_boolean_ttl_refused, "ttl", serde_json::json!(true));
runtime_field!(
    runtime_layer_string_refused,
    "capability_layer",
    serde_json::json!("144")
);
runtime_field!(
    runtime_negative_wrapper_layer_refused,
    "wrapper_layer",
    serde_json::json!(-1)
);
runtime_field!(
    runtime_nonfinite_created_at_refused,
    "created_at",
    serde_json::Value::Null
);
runtime_field!(
    runtime_rekey_timestamp_string_refused,
    "rekeyed_at",
    serde_json::json!("yesterday")
);
runtime_field!(
    runtime_handshake_list_refused,
    "handshake_secret",
    serde_json::json!([])
);
#[test]
fn runtime_mailbox_string_refused() {
    refused_field("deliveries", serde_json::json!("queue"));
    corrupt_service_mailbox_refuses_restoration();
}
runtime_field!(
    runtime_gap_marker_string_refused,
    "gap_end",
    serde_json::json!("false")
);
runtime_field!(
    runtime_short_visual_hash_refused,
    "initial_key_hash",
    serde_json::json!([1, 2, 3, 4, 5])
);
