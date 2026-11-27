#![forbid(unsafe_code)]

use std::future::{Future, poll_fn};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretChatManager, SecretTransport};
use kiaro_secret_chat::protocol::{
    ChatRole, KeyLifecycleState, SecretChat, decrypt_frame, key_fingerprint,
};
use kiaro_secret_chat::runtime_modes::{PersistentSecretChatManager, TransientSecretChatManager};
use kiaro_secret_chat::storage_backend::{
    FileStorage, MemoryStorage, PersistedChatRecord, SecretChatStore,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use tokio::sync::Notify;

const CHAT_ID: i32 = 73;
const OWNER: &str = "ttl-ordering-audit";
const CHAT_KEY: [u8; 256] = [0x57; 256];
const STORE_KEY: [u8; 32] = [0x68; 32];
const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct TransportState {
    frames: Mutex<Vec<Vec<u8>>>,
    block_next: AtomicBool,
    fail_next: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[derive(Clone, Default)]
struct Transport(Arc<TransportState>);

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        OWNER
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.0.frames.lock().unwrap().push(frame);
            self.0.entered.notify_one();
            if self.0.block_next.swap(false, Ordering::SeqCst) {
                self.0.release.notified().await;
            }
            if self.0.fail_next.swap(false, Ordering::SeqCst) {
                return Err(SecretChatError::Protocol("synthetic uncertain send"));
            }
            Ok(())
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        id: i64,
        random: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        self.send_encrypted(id, random, frame)
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

fn record() -> PersistedChatRecord {
    PersistedChatRecord::established(CHAT_ID, 92, 103, ChatRole::Acceptor, CHAT_KEY).unwrap()
}

fn peer_ttl(ttl: i32) -> Vec<u8> {
    let mut peer = SecretChat::new(
        i64::from(CHAT_ID),
        92,
        103,
        "synthetic-ttl-peer",
        ChatRole::Initiator,
    );
    peer.adopt_key(&CHAT_KEY).unwrap();
    peer.prepare_service_action(&json!({"id": 0xa1733aec_u32, "fields": {"ttl_seconds": ttl}}))
        .unwrap()
        .frame
}

fn encoded_ttl(frame: &[u8]) -> i64 {
    let body = decrypt_frame(&CHAT_KEY, frame, 8).unwrap();
    let decoded = kiaro_secret_chat::ported::schema_secret_tl::decode_object(
        hex::encode(body),
        "DecryptedMessageLayer".into(),
        0,
    )
    .unwrap();
    decoded["object"]["fields"]["message"]["fields"]["ttl"]
        .as_i64()
        .unwrap()
}

async fn persistent(
    path: &Path,
    transport: Transport,
) -> PersistentSecretChatManager<Transport, FileStorage> {
    PersistentSecretChatManager::open(
        transport,
        FileStorage::open(path, STORE_KEY, OWNER).unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn persistent_queued_send_uses_ttl_at_sequence_reservation() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("persistent-ttl-order.enc");
    let transport = Transport::default();
    let manager = Arc::new(persistent(&path, transport.clone()).await);
    manager.register_and_persist(record()).await.unwrap();
    transport.0.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let first = tokio::spawn(async move { sending.send_text_with_id(CHAT_ID, 101, "first").await });
    tokio::time::timeout(WAIT, transport.0.entered.notified())
        .await
        .unwrap();

    let second = manager.send_text_with_id(CHAT_ID, 102, "queued after first");
    tokio::pin!(second);
    poll_fn(|cx| {
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    tokio::time::timeout(WAIT, manager.handle_incoming(CHAT_ID, &peer_ttl(17)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manager.snapshot_chat(CHAT_ID).await.unwrap().ttl, 17);
    transport.0.release.notify_one();
    let first = tokio::time::timeout(WAIT, first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(WAIT, second).await.unwrap().unwrap();
    assert_eq!(encoded_ttl(&first), 0);
    assert_eq!(
        encoded_ttl(&second),
        17,
        "queued persistent send used a TTL captured before its sequence reservation"
    );
}

#[tokio::test]
async fn transient_queued_send_uses_ttl_at_sequence_reservation() {
    let transport = Transport::default();
    let manager = Arc::new(
        TransientSecretChatManager::open_after_restart(
            transport.clone(),
            MemoryStorage::new(OWNER).unwrap(),
            ManagerLimits::default(),
        )
        .await
        .unwrap(),
    );
    manager.register_chat(record()).await.unwrap();
    transport.0.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let first = tokio::spawn(async move { sending.send_text(CHAT_ID, "first").await });
    tokio::time::timeout(WAIT, transport.0.entered.notified())
        .await
        .unwrap();

    let second = manager.send_text(CHAT_ID, "queued after first");
    tokio::pin!(second);
    poll_fn(|cx| {
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    tokio::time::timeout(WAIT, manager.handle_incoming(CHAT_ID, &peer_ttl(17)))
        .await
        .unwrap()
        .unwrap();
    transport.0.release.notify_one();
    let first = tokio::time::timeout(WAIT, first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(WAIT, second).await.unwrap().unwrap();
    assert_eq!(encoded_ttl(&first), 0);
    assert_eq!(
        encoded_ttl(&second),
        17,
        "queued transient send used a TTL captured before its sequence reservation"
    );
}

#[tokio::test]
async fn persistent_same_id_retry_preserves_original_ttl_after_update_and_restart() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("persistent-ttl-retry.enc");
    let transport = Transport::default();
    let manager = persistent(&path, transport.clone()).await;
    manager.register_and_persist(record()).await.unwrap();
    transport.0.fail_next.store(true, Ordering::SeqCst);
    assert!(matches!(
        manager
            .send_text_with_id(CHAT_ID, 201, "retained text")
            .await,
        Err(SecretChatError::SendPending { .. })
    ));
    let original = transport.0.frames.lock().unwrap()[0].clone();
    assert_eq!(encoded_ttl(&original), 0);
    manager
        .handle_incoming(CHAT_ID, &peer_ttl(17))
        .await
        .unwrap();
    assert_eq!(manager.snapshot_chat(CHAT_ID).await.unwrap().ttl, 17);
    drop(manager);

    let reopened = persistent(&path, transport).await;
    let repeated = reopened
        .send_text_with_id(CHAT_ID, 201, "retained text")
        .await;
    assert!(
        repeated.is_ok(),
        "the same caller request became a different identity after an implicit TTL change: {repeated:?}"
    );
    assert_eq!(repeated.unwrap(), original);
    assert!(
        reopened
            .send_text_with_id(CHAT_ID, 201, "changed text")
            .await
            .is_err(),
        "retry recognition must still reject changed plaintext"
    );
}

#[tokio::test]
async fn explicit_ram_ttl_and_retry_identity_remain_explicit() {
    let manager = SecretChatManager::new(Arc::new(Transport::default()));
    manager
        .register_established_chat(i64::from(CHAT_ID), 92, 103, ChatRole::Acceptor, &CHAT_KEY)
        .await
        .unwrap();
    manager
        .receive_frames(i64::from(CHAT_ID), &peer_ttl(17))
        .await
        .unwrap();
    let original = manager
        .send_text(i64::from(CHAT_ID), 301, "explicit", 23)
        .await
        .unwrap();
    assert_eq!(encoded_ttl(&original), 23);
    assert_eq!(
        manager
            .send_text(i64::from(CHAT_ID), 301, "explicit", 23)
            .await
            .unwrap(),
        original
    );
    assert!(
        manager
            .send_text(i64::from(CHAT_ID), 301, "explicit", 24)
            .await
            .is_err()
    );
    assert!(
        manager
            .send_text(i64::from(CHAT_ID), 301, "changed", 23)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn implicit_ttl_is_selected_after_pending_outbox_drain() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("pending-drain-ttl.enc");
    let transport = Transport::default();
    let manager = Arc::new(persistent(&path, transport.clone()).await);
    manager.register_and_persist(record()).await.unwrap();
    transport.0.fail_next.store(true, Ordering::SeqCst);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 401, "earlier pending")
            .await
            .is_err()
    );
    // Consume the notification from the deliberately failed first attempt.
    tokio::time::timeout(WAIT, transport.0.entered.notified())
        .await
        .unwrap();
    transport.0.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let next = tokio::spawn(async move {
        sending
            .send_text_with_id(CHAT_ID, 402, "new after draining")
            .await
    });
    tokio::time::timeout(WAIT, transport.0.entered.notified())
        .await
        .unwrap();
    manager
        .handle_incoming(CHAT_ID, &peer_ttl(17))
        .await
        .unwrap();
    transport.0.release.notify_one();
    let next = tokio::time::timeout(WAIT, next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        encoded_ttl(&next),
        17,
        "TTL was selected before an older pending RPC finished draining"
    );
    let frames = transport.0.frames.lock().unwrap();
    assert_eq!(
        frames[0], frames[1],
        "draining changed the older ciphertext"
    );
}

#[derive(Clone, Copy)]
enum IdentityFormat {
    Legacy,
    Conversation,
}

async fn retained_store_fixture(
    path: &Path,
    format: IdentityFormat,
    ttl: i32,
    retire_original_key: bool,
) -> (Transport, Vec<u8>, serde_json::Value) {
    let transport = Transport::default();
    let manager = persistent(path, transport.clone()).await;
    manager.register_and_persist(record()).await.unwrap();
    transport.0.fail_next.store(true, Ordering::SeqCst);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 501, "legacy retained")
            .await
            .is_err()
    );
    let original = transport.0.frames.lock().unwrap()[0].clone();
    if ttl != 0 {
        manager
            .handle_incoming(CHAT_ID, &peer_ttl(ttl))
            .await
            .unwrap();
    }
    let mut record = manager.snapshot_chat(CHAT_ID).await.unwrap();
    assert_eq!(record.pending_outbox.len(), 1);
    drop(manager);

    let replacement_key = [0x79; 256];
    if retire_original_key {
        // Represent a retained old frame after its encryption key has retired.
        record.auth_key = replacement_key;
        record.key_fingerprint = key_fingerprint(&replacement_key).unwrap();
        record.rekey_state = KeyLifecycleState::Active {
            fingerprint: record.key_fingerprint,
        };
    }
    let mut stored = serde_json::to_value(&record).unwrap();
    if let IdentityFormat::Legacy = format {
        let mut hash = Sha256::new();
        hash.update(b"secret-chat-text\0");
        hash.update(0i64.to_le_bytes());
        hash.update(b"legacy retained");
        stored["outbox"][0]["identity"] = json!(hash.finalize().to_vec());
    }
    stored["outbox"][0]["pending"] = json!(true);
    if retire_original_key {
        stored["protocol"]["active_key"] = json!(replacement_key.to_vec());
        stored["protocol"]["key_lifecycle"] = serde_json::to_value(record.rekey_state).unwrap();
    }
    let identity = stored["outbox"][0]["identity"].clone();
    let saved: PersistedChatRecord = serde_json::from_value(stored).unwrap();
    FileStorage::open(path, STORE_KEY, OWNER)
        .unwrap()
        .save(&saved)
        .unwrap();
    (transport, original, identity)
}

#[tokio::test]
async fn legacy_retry_recovers_original_ttl_without_rewriting_identity_or_ciphertext() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("legacy-available-key.enc");
    let (transport, original, identity) =
        retained_store_fixture(&path, IdentityFormat::Legacy, 17, false).await;
    let manager = persistent(&path, transport).await;
    let retry = manager
        .send_text_with_id(CHAT_ID, 501, "legacy retained")
        .await
        .unwrap();
    assert_eq!(retry, original);
    let snapshot = serde_json::to_value(manager.snapshot_chat(CHAT_ID).await.unwrap()).unwrap();
    assert!(
        snapshot["outbox"][0]["identity"] == identity,
        "legacy digest was rewritten"
    );
    assert_eq!(encoded_ttl(&retry), 0);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 501, "different content")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_unavailable_key_refuses_reinterpretation_and_keeps_exact_resend() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("legacy-retired-key.enc");
    let (transport, original, identity) =
        retained_store_fixture(&path, IdentityFormat::Legacy, 17, true).await;
    let manager = persistent(&path, transport.clone()).await;
    let before = std::fs::read(&path).unwrap();
    let sent_before = transport.0.frames.lock().unwrap().len();
    let error = manager
        .send_text_with_id(CHAT_ID, 501, "legacy retained")
        .await
        .unwrap_err();
    assert!(
        matches!(&error, SecretChatError::ParameterRejected { reason, .. } if reason.contains("retry_pending"))
    );
    assert!(
        std::fs::read(&path).unwrap() == before,
        "refused legacy verification changed the store"
    );
    assert_eq!(transport.0.frames.lock().unwrap().len(), sent_before);
    assert_eq!(manager.retry_pending(CHAT_ID).await.unwrap(), 1);
    assert_eq!(
        transport.0.frames.lock().unwrap().last().unwrap(),
        &original
    );
    let snapshot = serde_json::to_value(manager.snapshot_chat(CHAT_ID).await.unwrap()).unwrap();
    assert!(
        snapshot["outbox"][0]["identity"] == identity,
        "legacy digest was rewritten"
    );
    assert!(snapshot["protocol"]["previous_key"].is_null());
}

#[tokio::test]
async fn legacy_matching_ttl_digest_does_not_require_a_retired_key() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("legacy-retired-key-same-ttl.enc");
    let (transport, original, identity) =
        retained_store_fixture(&path, IdentityFormat::Legacy, 0, true).await;
    let manager = persistent(&path, transport).await;
    assert_eq!(
        manager
            .send_text_with_id(CHAT_ID, 501, "legacy retained")
            .await
            .unwrap(),
        original
    );
    let snapshot = serde_json::to_value(manager.snapshot_chat(CHAT_ID).await.unwrap()).unwrap();
    assert!(
        snapshot["outbox"][0]["identity"] == identity,
        "legacy digest was rewritten"
    );
}

#[tokio::test]
async fn conversation_identity_retries_after_original_key_retirement_and_ttl_change() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("conversation-retired-key.enc");
    let (transport, original, identity) =
        retained_store_fixture(&path, IdentityFormat::Conversation, 17, true).await;
    let manager = persistent(&path, transport).await;
    assert_eq!(
        manager
            .send_text_with_id(CHAT_ID, 501, "legacy retained")
            .await
            .unwrap(),
        original
    );
    let snapshot = serde_json::to_value(manager.snapshot_chat(CHAT_ID).await.unwrap()).unwrap();
    assert!(
        snapshot["outbox"][0]["identity"] == identity,
        "conversation digest was rewritten"
    );
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 501, "different content")
            .await
            .is_err()
    );
}
