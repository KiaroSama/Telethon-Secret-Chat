//! Restore and sequence regressions with real native storage and manager APIs.
#![allow(clippy::unwrap_used)]
#[path = "support_protocol/foundation.rs"]
mod foundation;
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
mod support_protocol;
use kiaro_secret_chat::{
    Serializable,
    chat::ChatState,
    crypto::SharedKey,
    enums, framing,
    manager::{Options, SecretChatManager},
    sequence,
    storage::{FileStorage, State, Storage},
    types,
};
use serde_json::{Value, json};
use support_protocol::*;

fn refuse(mut change: impl FnMut(&mut Value)) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    store.commit(&state).unwrap();
    drop(store);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut value);
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let (transport, _) = support::Fake::pair();
    let world = transport.world.clone();
    let result = SecretChatManager::new(
        transport,
        FileStorage::open(&path).unwrap(),
        "11".repeat(32),
        Options::default(),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("Invalid storage installed a manager"),
    };
    let text = format!("{error} {error:?}");
    assert!(text.len() < 512);
    assert!(!text.contains(&"07".repeat(16)));
    let world = world.lock().unwrap();
    assert!(world.sent.is_empty() && world.requests.is_empty() && world.discards.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
macro_rules! corrupt {
    ($name:ident,$field:literal,$value:expr) => {
        #[test]
        fn $name() {
            refuse(|state| state["chats"]["7"][$field] = $value);
        }
    };
}
// Fingerprints are derived from a validated SharedKey in Rust; a separate,
// inconsistent fingerprint is deliberately not a supported persisted field.
corrupt!(
    independent_key_fingerprint_cannot_disagree,
    "key_fingerprint",
    json!(123)
);
corrupt!(unknown_chat_state_refuses, "state", json!("half-open"));
corrupt!(ready_without_key_refuses, "key", Value::Null);
corrupt!(negative_counter_refuses, "out_seq_no", json!(-1));
corrupt!(boolean_counter_refuses, "in_seq_no", json!(true));
corrupt!(
    short_pending_key_refuses,
    "exchange",
    json!({"role":"accepted","id":1,"key":"00".repeat(10)})
);
corrupt!(
    long_previous_key_refuses,
    "previous_key",
    json!("00".repeat(257))
);
corrupt!(string_chat_identity_refuses, "id", json!("7"));
corrupt!(malformed_resend_span_refuses, "resend_due", json!([1]));
#[test]
fn record_under_another_identity_refuses() {
    refuse(|state| {
        let row = state["chats"].as_object_mut().unwrap().remove("7").unwrap();
        state["chats"]["8"] = row;
    });
}
#[test]
fn one_corrupt_record_refuses_the_entire_snapshot() {
    refuse(|state| {
        let mut valid = state["chats"]["7"].clone();
        valid["id"] = json!(11);
        state["chats"]["11"] = valid;
        state["chats"]["7"]["key"] = json!("00".repeat(100));
    });
}
#[tokio::test]
async fn valid_native_legacy_optional_fields_still_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    for id in [7, 21, 22] {
        let mut c = chat(true, 0);
        c.id = id;
        if id == 22 {
            c.close();
        }
        state.chats.insert(id, c);
    }
    store.commit(&state).unwrap();
    drop(store);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for field in [
        "gap_end",
        "new_key_confirmed",
        "initial_key_hash",
        "handshake_secret",
        "peer_public",
        "created_at",
        "admin_id",
        "participant_id",
        "closed_reason",
    ] {
        value["chats"]["21"].as_object_mut().unwrap().remove(field);
    }
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let (transport, _) = support::Fake::pair();
    let world = transport.world.clone();
    let manager = SecretChatManager::new(
        transport,
        FileStorage::open(&path).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    assert_eq!(
        manager
            .list()
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![7, 21, 22]
    );
    assert_eq!(manager.status(7).unwrap().state, ChatState::Ready);
    assert_eq!(manager.status(22).unwrap().state, ChatState::Closed);
    assert!(world.lock().unwrap().sent.is_empty());
    manager.stop().await.unwrap();
}
fn wrong_key(length: usize) {
    let frame = frame(&envelope(0, 0, 144, true).body);
    assert!(SharedKey::from_slice(&vec![0; length]).is_err());
    // Neither frame API accepts an unvalidated byte string: both require SharedKey.
    assert!(
        kiaro_secret_chat::crypto::decrypt_frame(
            &key(),
            &frame,
            kiaro_secret_chat::crypto::Direction::Responder
        )
        .is_ok()
    );
}
#[test]
fn key_width_255_refused_before_cipher() {
    wrong_key(255);
}
#[test]
fn key_width_257_refused_before_cipher() {
    wrong_key(257);
}
fn permutation(order: [i32; 4]) {
    let mut c = chat(true, 0);
    let mut delivered = Vec::new();
    for raw in order {
        delivered.extend(ids(
            sequence::accept(&mut c, envelope(raw, 0, 144, true)).unwrap()
        ));
    }
    assert_eq!(delivered, vec![0, 1, 2, 3]);
    assert_eq!(c.in_seq_no, 4);
    assert_ready(&c);
    assert!(c.gaps.is_empty());
    assert!(c.gap_end.is_none());
    assert!(c.resend_due.is_none());
}
macro_rules! arrival {
    ($name:ident,$order:expr) => {
        #[test]
        fn $name() {
            permutation($order);
        }
    };
}
arrival!(arrival_order_00, [0, 1, 2, 3]);
arrival!(arrival_order_01, [0, 1, 3, 2]);
arrival!(arrival_order_02, [0, 2, 1, 3]);
arrival!(arrival_order_03, [0, 2, 3, 1]);
arrival!(arrival_order_04, [0, 3, 1, 2]);
arrival!(arrival_order_05, [0, 3, 2, 1]);
arrival!(arrival_order_06, [1, 0, 2, 3]);
arrival!(arrival_order_07, [1, 0, 3, 2]);
arrival!(arrival_order_08, [1, 2, 0, 3]);
arrival!(arrival_order_09, [1, 2, 3, 0]);
arrival!(arrival_order_10, [1, 3, 0, 2]);
arrival!(arrival_order_11, [1, 3, 2, 0]);
arrival!(arrival_order_12, [2, 0, 1, 3]);
arrival!(arrival_order_13, [2, 0, 3, 1]);
arrival!(arrival_order_14, [2, 1, 0, 3]);
arrival!(arrival_order_15, [2, 1, 3, 0]);
arrival!(arrival_order_16, [2, 3, 0, 1]);
arrival!(arrival_order_17, [2, 3, 1, 0]);
arrival!(arrival_order_18, [3, 0, 1, 2]);
arrival!(arrival_order_19, [3, 0, 2, 1]);
arrival!(arrival_order_20, [3, 1, 0, 2]);
arrival!(arrival_order_21, [3, 1, 2, 0]);
arrival!(arrival_order_22, [3, 2, 0, 1]);
arrival!(arrival_order_23, [3, 2, 1, 0]);
#[tokio::test]
async fn due_key_rekeys_before_service_only_traffic() {
    foundation::key_visualization_survives_rekey();
    foundation::fallback_adoption_queues_new_key_confirmation_with_an_open_gap();

    let mut c = seeded::chat();
    c.messages_since_rekey = 101;
    let (manager, _store, world, id) = seeded::ready_chat(c, Options::default()).await;
    manager.mark_read(id, vec![1]).await.unwrap();
    let actions = world
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter_map(
            |e| match framing::unwrap(&e.outgoing.body).unwrap().message {
                enums::DecryptedMessage::Service(s) => Some(s.action),
                _ => None,
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 2);
    assert!(matches!(
        actions[0],
        enums::DecryptedMessageAction::RequestKey(_)
    ));
    assert!(matches!(
        actions[1],
        enums::DecryptedMessageAction::ReadMessages(_)
    ));
    assert_eq!(manager.status(id).unwrap().state, ChatState::Rekeying);
    manager.stop().await.unwrap();
}
#[tokio::test]
async fn peer_rekey_without_persisted_dh_closes_instead_of_pinning_mailbox() {
    let mut c = seeded::chat();
    c.dh = None;
    let (manager, store, world, id) = seeded::ready_chat(c, Options::default()).await;
    let action = types::DecryptedMessageActionRequestKey {
        exchange_id: 1,
        g_a: vec![0x80; 256],
    }
    .into();
    let body = framing::wrap(
        types::DecryptedMessageService {
            random_id: 77,
            action,
        }
        .into(),
        144,
        1,
        0,
    )
    .unwrap()
    .to_bytes();
    let update: grammers_tl_types::enums::Update =
        grammers_tl_types::types::UpdateNewEncryptedMessage {
            message: grammers_tl_types::types::EncryptedMessageService {
                random_id: 77,
                chat_id: id,
                date: 1,
                bytes: frame(&body),
            }
            .into(),
            qts: 1,
        }
        .into();
    manager.handle_update(&update).await;
    let closed = store.load().unwrap().unwrap().chats.remove(&id).unwrap();
    assert_eq!(closed.state, ChatState::Closed);
    assert!(closed.key.is_none() && closed.service_deliveries.is_empty());
    assert_eq!(
        closed.closed_reason,
        Some(kiaro_secret_chat::error::CloseReason::MissingDh)
    );
    assert_eq!(world.lock().unwrap().discards, vec![(id, false)]);
    manager.stop().await.unwrap();
}
#[tokio::test]
async fn only_closed_chat_can_be_forgotten() {
    foundation::paused_chat_refuses_sends_until_explicit_resume();
    foundation::stopping_refuses_further_effects();
    foundation::closed_tombstone_prevents_revival();

    let (manager, store, _, id) = seeded::ready(Options::default()).await;
    assert!(manager.forget(id).await.is_err());
    assert!(store.load().unwrap().unwrap().chats.contains_key(&id));
    manager.close(id).await.unwrap();
    manager.forget(id).await.unwrap();
    assert!(manager.list().unwrap().is_empty());
    assert!(!store.load().unwrap().unwrap().chats.contains_key(&id));
    manager.stop().await.unwrap();
}

async fn failed_local_create(discard_fails: bool) {
    let (transport, _) = support::Fake::pair();
    let world = transport.world.clone();
    let store = support::TestStorage::default();
    let observer = store.clone();
    world.lock().unwrap().fail_discard = discard_fails;
    world.lock().unwrap().request_hook = Some(Box::new(move |_| {
        Box::pin(async move {
            observer
                .failures
                .store(1, std::sync::atomic::Ordering::Release);
        })
    }));
    let manager = SecretChatManager::new(
        transport,
        store.clone(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let failure = manager.create(support::peer(2000)).await.unwrap_err();
    assert!(matches!(failure, kiaro_secret_chat::error::Error::Storage));
    let world = world.lock().unwrap();
    assert_eq!(world.requests.len(), 1);
    let id = *world.requests.first_key_value().unwrap().0;
    assert_eq!(world.discards, vec![(id, false)]);
    assert!(manager.list().unwrap().is_empty());
    assert!(store.load().unwrap().unwrap().chats.is_empty());
}
#[tokio::test]
async fn create_save_failure_discards_server_chat() {
    failed_local_create(false).await;
}
#[tokio::test]
async fn create_save_failure_preserves_original_error_if_discard_fails() {
    failed_local_create(true).await;
}
