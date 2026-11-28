#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::ingress::{
    DurableIngressController, EncryptedIngressStore, IngressLimits, IngressOrigin, IngressUpdate,
};
use kiaro_secret_chat::manager::{
    BoxFuture, CoreSecretChatManager, ManagerLimits, SecretTransport,
};
use kiaro_secret_chat::protocol::ChatRole;
use kiaro_secret_chat::runtime_modes::{PersistentSecretChatManager, StrongSecretChatManager};
use kiaro_secret_chat::storage_backend::{FileStorage, PersistedChatRecord};
use tempfile::tempdir;

const CHAT: i32 = 7;
const OWNER: &str = "ingress-audit-authorization";
const STORE_KEY: [u8; 32] = [0x61; 32];
const CHAT_KEY: [u8; 256] = [0x42; 256];

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

type Mode = StrongSecretChatManager<Transport, FileStorage>;

async fn open(directory: &Path) -> Mode {
    open_with_limits(directory, ManagerLimits::default()).await
}

async fn open_with_limits(directory: &Path, limits: ManagerLimits) -> Mode {
    let persistent = PersistentSecretChatManager::open(
        Transport,
        FileStorage::open(directory.join("chat.store"), STORE_KEY, OWNER).unwrap(),
        limits,
    )
    .await
    .unwrap();
    let ingress = DurableIngressController::restore(
        OWNER,
        EncryptedIngressStore::open(directory.join("ingress.store"), STORE_KEY).unwrap(),
        IngressLimits::default(),
    )
    .unwrap();
    Mode::new(persistent, ingress).unwrap()
}

async fn register(mode: &Mode) {
    mode.register_and_persist(
        PersistedChatRecord::established(CHAT, 101, 202, ChatRole::Acceptor, CHAT_KEY).unwrap(),
    )
    .await
    .unwrap();
}

async fn sender() -> CoreSecretChatManager<Transport> {
    let sender = CoreSecretChatManager::new(Arc::new(Transport));
    sender
        .register_established_chat(7, 101, 202, ChatRole::Initiator, &CHAT_KEY)
        .await
        .unwrap();
    sender
}

async fn capture(mode: &Mode, order: i64, payload: Vec<u8>) {
    mode.ingest_before_ack(IngressUpdate {
        stream_id: Mode::stream_id(CHAT),
        order,
        provenance: IngressOrigin::VerifiedTransport,
        associated_chat_id: Some(i64::from(CHAT)),
        payload,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn strong_repeated_apply_preserves_the_unsettled_consumer_batch() {
    let directory = tempdir().unwrap();
    let mode = open(directory.path()).await;
    register(&mode).await;
    capture(
        &mode,
        1,
        sender()
            .await
            .send_text(7, 17, "awaiting consumer", 0)
            .await
            .unwrap(),
    )
    .await;
    let original = mode.apply_delivery(CHAT, 1).await.unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].text.as_deref(), Some("awaiting consumer"));
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 0);
    let repeated = mode.apply_delivery(CHAT, 1).await.unwrap();
    assert_eq!(
        repeated, original,
        "an unsettled obligation must retain its accepted result"
    );
    assert!(mode.acknowledge_delivery(CHAT, 1).await.unwrap());
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 1);
    assert!(
        mode.list_pending_deliveries(CHAT, 10, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn strong_restart_replays_the_result_before_consumer_acknowledgement() {
    let directory = tempdir().unwrap();
    let mode = open(directory.path()).await;
    register(&mode).await;
    capture(
        &mode,
        1,
        sender()
            .await
            .send_text(7, 19, "recover this delivery", 0)
            .await
            .unwrap(),
    )
    .await;
    let original = mode.apply_delivery(CHAT, 1).await.unwrap();
    assert_eq!(original.len(), 1);
    drop(mode);

    let mode = open(directory.path()).await;
    let replay = mode.replay_pending().await.unwrap();
    assert!(replay.chat_errors.is_empty());
    assert_eq!(replay.applied.len(), 1);
    assert_eq!(replay.applied[0].0, CHAT);
    assert_eq!(replay.applied[0].1, 1);
    assert_eq!(
        replay.applied[0].2, original,
        "restart must not turn owed payload into an empty batch"
    );
    assert!(mode.acknowledge_delivery(CHAT, 1).await.unwrap());
    drop(mode);

    let mode = open(directory.path()).await;
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 1);
    assert!(mode.replay_pending().await.unwrap().applied.is_empty());
}

#[tokio::test]
async fn strong_restart_retains_every_message_recovered_by_a_gap_filling_delivery() {
    let directory = tempdir().unwrap();
    let mode = open(directory.path()).await;
    register(&mode).await;
    let peer = sender().await;
    let first = peer.send_text(7, 21, "first", 0).await.unwrap();
    let second = peer.send_text(7, 23, "second", 0).await.unwrap();
    capture(&mode, 1, second).await;
    assert!(mode.apply_delivery(CHAT, 1).await.unwrap().is_empty());
    assert!(mode.acknowledge_delivery(CHAT, 1).await.unwrap());
    capture(&mode, 2, first).await;
    let original = mode.apply_delivery(CHAT, 2).await.unwrap();
    assert_eq!(
        original
            .iter()
            .map(|m| m.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("first"), Some("second")]
    );
    drop(mode);

    let mode = open(directory.path()).await;
    let replay = mode.replay_pending().await.unwrap();
    assert!(replay.chat_errors.is_empty());
    assert_eq!(replay.applied.len(), 1);
    assert_eq!(replay.applied[0].1, 2);
    assert_eq!(
        replay.applied[0].2, original,
        "the complete recovered batch remains owed until ACK"
    );
    assert!(mode.acknowledge_delivery(CHAT, 2).await.unwrap());
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 2);
}

#[tokio::test]
async fn committed_acknowledgement_cleans_leftover_receipts_before_the_next_bounded_delivery() {
    let directory = tempdir().unwrap();
    let limits = ManagerLimits {
        max_retained_messages: 1,
        max_retained_bytes: 64 * 1024,
    };
    let mode = open_with_limits(directory.path(), limits).await;
    register(&mode).await;
    let peer = sender().await;
    capture(
        &mode,
        1,
        peer.send_text(7, 31, "first bounded receipt", 0)
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(mode.apply_delivery(CHAT, 1).await.unwrap().len(), 1);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("chat.store.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(mode.acknowledge_delivery(CHAT, 1).await.is_err());
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 1);
    drop(lock);
    drop(mode);
    let mode = open_with_limits(directory.path(), limits).await;
    assert!(!mode.acknowledge_delivery(CHAT, 1).await.unwrap());
    capture(
        &mode,
        2,
        peer.send_text(7, 32, "second bounded receipt", 0)
            .await
            .unwrap(),
    )
    .await;
    let messages = mode.apply_delivery(CHAT, 2).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text.as_deref(), Some("second bounded receipt"));
    assert!(mode.acknowledge_delivery(CHAT, 2).await.unwrap());
}

#[tokio::test]
async fn receive_budget_refusal_preserves_protocol_and_ingress_for_a_larger_reopen() {
    let directory = tempdir().unwrap();
    let limits = ManagerLimits {
        max_retained_messages: 1,
        max_retained_bytes: 64 * 1024,
    };
    let mode = open_with_limits(directory.path(), limits).await;
    register(&mode).await;
    let peer = sender().await;
    let first = peer.send_text(7, 41, "first", 0).await.unwrap();
    let second = peer.send_text(7, 42, "second", 0).await.unwrap();
    capture(&mode, 1, second).await;
    assert!(mode.apply_delivery(CHAT, 1).await.unwrap().is_empty());
    assert!(mode.acknowledge_delivery(CHAT, 1).await.unwrap());
    capture(&mode, 2, first).await;
    assert!(mode.apply_delivery(CHAT, 2).await.is_err());
    assert_eq!(mode.applied_cursor(CHAT).await.unwrap(), 1);
    assert_eq!(
        mode.list_pending_deliveries(CHAT, 10, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    drop(mode);
    let mode = open(directory.path()).await;
    let messages = mode.apply_delivery(CHAT, 2).await.unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("first"), Some("second")]
    );
    assert!(mode.acknowledge_delivery(CHAT, 2).await.unwrap());
}
