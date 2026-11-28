#![forbid(unsafe_code)]

use std::sync::Arc;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{
    BoxFuture, CoreSecretChatManager, ManagerLimits, SecretTransport,
};
use kiaro_secret_chat::ported::schema_secret_tl;
use kiaro_secret_chat::protocol::{ChatRole, key_fingerprint};
use kiaro_secret_chat::runtime_modes::PersistentSecretChatManager;
use kiaro_secret_chat::storage_backend::{MemoryStorage, PersistedChatRecord, SecretChatStore};
use serde_json::{Value, json};

const OWNER: &str = "snapshot-audit-authorization";
const KEY: [u8; 256] = [0x42; 256];

#[derive(Clone)]
struct Transport;

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        OWNER
    }

    fn send_encrypted<'a>(
        &'a self,
        _chat_id: i64,
        _random_id: i64,
        _frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        self.send_encrypted(chat_id, random_id, frame)
    }

    fn discard_encryption<'a>(
        &'a self,
        _chat_id: i64,
        _delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

type Mode = PersistentSecretChatManager<Transport, MemoryStorage>;

async fn new_receiver() -> Mode {
    let mode = Mode::open(
        Transport,
        MemoryStorage::new(OWNER).unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap();
    mode.register_and_persist(
        PersistedChatRecord::established(7, 101, 202, ChatRole::Acceptor, KEY).unwrap(),
    )
    .await
    .unwrap();
    mode
}

async fn sender() -> CoreSecretChatManager<Transport> {
    let sender = CoreSecretChatManager::new(Arc::new(Transport));
    sender
        .register_established_chat(7, 101, 202, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    sender
}

async fn reopen(record: Value) -> Result<Mode, SecretChatError> {
    let record: PersistedChatRecord = serde_json::from_value(record).unwrap();
    let store = MemoryStorage::new(OWNER).unwrap();
    store.save(&record)?;
    Mode::open(Transport, store, ManagerLimits::default()).await
}

async fn assert_corrupt(record: Value) {
    assert!(
        matches!(
            reopen(record).await,
            Err(SecretChatError::StoreCorrupt { .. })
        ),
        "an inconsistent protocol checkpoint must fail before installing live state"
    );
}

async fn gapped_snapshot() -> (Value, Vec<u8>) {
    let mode = new_receiver().await;
    let peer = sender().await;
    let first = peer.send_text(7, 1, "first", 0).await.unwrap();
    let second = peer.send_text(7, 2, "second", 0).await.unwrap();
    assert!(mode.handle_incoming(7, &second).await.unwrap().is_empty());
    let record = serde_json::to_value(mode.snapshot_chat(7).await.unwrap()).unwrap();
    assert_eq!(
        record["protocol"]["pending_frames"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    (record, first)
}

#[tokio::test]
async fn phantom_pending_identity_cannot_suppress_a_valid_first_message() {
    let mode = new_receiver().await;
    let mut record = serde_json::to_value(mode.snapshot_chat(7).await.unwrap()).unwrap();
    record["protocol"]["pending_frame_keys"] = json!({"0": key_fingerprint(&KEY).unwrap()});
    match reopen(record).await {
        Err(SecretChatError::StoreCorrupt { .. }) => (),
        Ok(restored) => {
            let frame = sender()
                .await
                .send_text(7, 1, "must not disappear", 0)
                .await
                .unwrap();
            let batch = restored.handle_incoming(7, &frame).await.unwrap();
            assert_eq!(
                batch.len(),
                1,
                "phantom replay metadata silently dropped authentic input"
            );
            panic!("the inconsistent checkpoint should have been refused");
        }
        Err(error) => panic!("unexpected restore error: {error}"),
    }
}

#[tokio::test]
async fn persisted_peer_ack_cannot_exceed_the_local_sent_counter() {
    let mode = new_receiver().await;
    let mut record = serde_json::to_value(mode.snapshot_chat(7).await.unwrap()).unwrap();
    record["protocol"]["peer_in_seq_no"] = json!(1);
    assert_corrupt(record).await;
}

#[tokio::test]
async fn persisted_counters_cannot_exceed_the_wire_integer_range() {
    let mode = new_receiver().await;
    let original = serde_json::to_value(mode.snapshot_chat(7).await.unwrap()).unwrap();
    for field in ["in_seq_no", "out_seq_no"] {
        let mut record = original.clone();
        record[field] = json!(i32::MAX);
        record["protocol"][field] = json!(i32::MAX);
        assert_corrupt(record).await;
    }
    let mut record = original;
    record["out_seq_no"] = json!(1 << 30);
    record["protocol"]["out_seq_no"] = json!(1 << 30);
    record["protocol"]["peer_in_seq_no"] = json!(1 << 30);
    assert_corrupt(record).await;
}

#[tokio::test]
async fn the_final_consumed_wire_index_can_be_restored() {
    let mode = new_receiver().await;
    let mut record = serde_json::to_value(mode.snapshot_chat(7).await.unwrap()).unwrap();
    for field in ["in_seq_no", "out_seq_no"] {
        record[field] = json!(1 << 30);
        record["protocol"][field] = json!(1 << 30);
    }
    record["protocol"]["peer_in_seq_no"] = json!((1 << 30) - 1);
    reopen(record).await.unwrap();
}

#[tokio::test]
async fn pending_frames_require_matching_key_provenance() {
    let (mut record, _) = gapped_snapshot().await;
    record["protocol"]["pending_frame_keys"] = json!({});
    assert_corrupt(record).await;
}

#[tokio::test]
async fn pending_frames_cannot_be_duplicated_or_name_a_different_sequence() {
    let (record, _) = gapped_snapshot().await;
    let mut duplicate = record.clone();
    let frame = duplicate["protocol"]["pending_frames"][0].clone();
    duplicate["protocol"]["pending_frames"]
        .as_array_mut()
        .unwrap()
        .push(frame);
    assert_corrupt(duplicate).await;

    let mut wrong_index = record;
    wrong_index["protocol"]["pending_frames"][0]["seq_no"] = json!(999);
    assert_corrupt(wrong_index).await;
}

#[tokio::test]
async fn persisted_pending_frames_require_a_complete_valid_wrapper() {
    let (mut record, _) = gapped_snapshot().await;
    record["protocol"]["pending_frames"][0]["body"] = json!("00000000");
    assert_corrupt(record).await;
}

#[tokio::test]
async fn persisted_pending_window_is_subject_to_live_receive_limits() {
    let (mut record, _) = gapped_snapshot().await;
    let frame = &mut record["protocol"]["pending_frames"][0];
    let mut decoded = schema_secret_tl::decode_object(
        frame["body"].as_str().unwrap().to_owned(),
        "DecryptedMessageLayer".into(),
        0,
    )
    .unwrap();
    decoded["object"]["fields"]["out_seq_no"] = json!(2003);
    frame["body"] = json!(schema_secret_tl::encode_object(decoded["object"].clone()).unwrap());
    frame["seq_no"] = json!(1001);
    record["protocol"]["pending_frame_keys"] = json!({"1001": key_fingerprint(&KEY).unwrap()});
    record["protocol"]["gap_end"] = json!(1001);
    record["protocol"]["resend_due"] = json!([1, 2001]);
    assert_corrupt(record).await;
}

#[tokio::test]
async fn persisted_gap_metadata_cannot_hide_buffered_work() {
    let (mut record, _) = gapped_snapshot().await;
    record["protocol"]["gap_requested"] = json!(false);
    record["protocol"]["gap_end"] = Value::Null;
    record["protocol"]["resend_due"] = Value::Null;
    assert_corrupt(record).await;
}

#[tokio::test]
async fn a_valid_gap_snapshot_restores_and_drains_in_order() {
    let (record, first) = gapped_snapshot().await;
    let restored = reopen(record).await.unwrap();
    let batch = restored.handle_incoming(7, &first).await.unwrap();
    assert_eq!(
        batch
            .iter()
            .map(|message| message.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("first"), Some("second")]
    );
    let settled = serde_json::to_value(restored.snapshot_chat(7).await.unwrap()).unwrap();
    assert!(
        settled["protocol"]["pending_frames"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        settled["protocol"]["pending_frame_keys"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    reopen(settled).await.unwrap();
}
