#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, SecretChat, decrypt_frame, encrypt_frame, wrap_frame};
use kiaro_secret_chat::runtime_modes::{PersistentSecretChatManager, TransientSecretChatManager};
use kiaro_secret_chat::storage_backend::{
    FileStorage, MemoryStorage, PersistedChatRecord, SecretChatStore,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::Notify;

const CHAT_ID: i32 = 47;
const OWNER: &str = "persistence-audit-owner";
const STORE_KEY: [u8; 32] = [0x27; 32];
const CHAT_KEY: [u8; 256] = [0x39; 256];

#[derive(Default)]
struct DeliveryState {
    frames: Mutex<Vec<Vec<u8>>>,
    blocked: AtomicBool,
    fail: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[derive(Clone)]
struct Transport {
    path: PathBuf,
    state: Arc<DeliveryState>,
}

impl Transport {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            state: Arc::default(),
        }
    }
}

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        OWNER
    }

    fn send_encrypted<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            let store = FileStorage::open(&self.path, STORE_KEY, OWNER).unwrap();
            let checkpoint = store.get(i32::try_from(chat_id).unwrap()).unwrap().unwrap();
            let retained = checkpoint
                .pending_outbox
                .iter()
                .find(|entry| entry.random_id == random_id)
                .expect("transmission must have a durable retained frame");
            assert_eq!(retained.payload, frame);
            assert!(retained.attempts > 0);
            self.state.frames.lock().unwrap().push(frame);
            self.state.entered.notify_one();
            if self.state.blocked.swap(false, Ordering::SeqCst) {
                self.state.release.notified().await;
            }
            if self.state.fail.swap(false, Ordering::SeqCst) {
                return Err(SecretChatError::Protocol("synthetic transport failure"));
            }
            Ok(())
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        self.send_encrypted(chat_id, random_id, frame)
    }

    fn discard_encryption(
        &self,
        _chat_id: i64,
        _delete_history: bool,
    ) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

async fn open(
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

async fn register(manager: &PersistentSecretChatManager<Transport, FileStorage>) {
    manager
        .register_and_persist(
            PersistedChatRecord::established(CHAT_ID, 92, 103, ChatRole::Acceptor, CHAT_KEY)
                .unwrap(),
        )
        .await
        .unwrap();
}

fn incoming(sequence: i64, text: &str) -> Vec<u8> {
    let body = wrap_frame(
        &json!({"random_id": 1000 + sequence, "ttl": 0, "message": text}),
        46,
        0,
        sequence * 2 + 1,
    )
    .unwrap();
    encrypt_frame(&CHAT_KEY, &body, 0).unwrap()
}

fn lock_checkpoint(path: &Path) -> File {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_file_name(name))
        .unwrap();
    file.try_lock().unwrap();
    file
}

#[tokio::test]
async fn failed_receive_checkpoint_leaves_the_same_frame_deliverable() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("failed-receive.enc");
    let transport = Transport::new(&path);
    let manager = open(&path, transport.clone()).await;
    register(&manager).await;
    let frame = incoming(0, "must remain deliverable after a refused save");
    let lock = lock_checkpoint(&path);
    assert!(manager.handle_incoming(CHAT_ID, &frame).await.is_err());
    assert_eq!(manager.snapshot_chat(CHAT_ID).await.unwrap().in_seq_no, 0);
    drop(lock);
    let accepted = manager.handle_incoming(CHAT_ID, &frame).await.unwrap();
    assert_eq!(
        accepted.len(),
        1,
        "a failed checkpoint consumed the message"
    );
    assert_eq!(
        accepted[0].text.as_deref(),
        Some("must remain deliverable after a refused save")
    );
    drop(manager);
    let reopened = open(&path, transport).await;
    assert_eq!(reopened.snapshot_chat(CHAT_ID).await.unwrap().in_seq_no, 1);
    assert!(
        reopened
            .handle_incoming(CHAT_ID, &frame)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn failed_send_checkpoint_never_transmits_and_can_retry() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("failed-send.enc");
    let transport = Transport::new(&path);
    let manager = open(&path, transport.clone()).await;
    register(&manager).await;
    let lock = lock_checkpoint(&path);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 88, "retry after save")
            .await
            .is_err()
    );
    assert!(transport.state.frames.lock().unwrap().is_empty());
    drop(lock);
    let frame = manager
        .send_text_with_id(CHAT_ID, 88, "retry after save")
        .await
        .unwrap();
    assert_eq!(transport.state.frames.lock().unwrap().as_slice(), &[frame]);
    assert_eq!(manager.snapshot_chat(CHAT_ID).await.unwrap().out_seq_no, 1);
}

#[tokio::test]
async fn cancelled_send_reopens_with_the_exact_retained_ciphertext() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("cancelled-send.enc");
    let transport = Transport::new(&path);
    let manager = Arc::new(open(&path, transport.clone()).await);
    register(&manager).await;
    transport.state.blocked.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let task = tokio::spawn(async move {
        sending
            .send_text_with_id(CHAT_ID, 91, "cancelled transmission")
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), transport.state.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let original = transport.state.frames.lock().unwrap()[0].clone();
    drop(manager);
    let reopened = open(&path, transport.clone()).await;
    assert_eq!(reopened.retry_pending(CHAT_ID).await.unwrap(), 1);
    {
        let frames = transport.state.frames.lock().unwrap();
        assert_eq!(frames.as_slice(), &[original.clone(), original]);
    }
    assert_eq!(reopened.snapshot_chat(CHAT_ID).await.unwrap().out_seq_no, 1);
}

#[tokio::test]
async fn failed_transport_reopens_with_the_exact_retained_ciphertext() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("failed-transport.enc");
    let transport = Transport::new(&path);
    let manager = open(&path, transport.clone()).await;
    register(&manager).await;
    transport.state.fail.store(true, Ordering::SeqCst);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 92, "lost transport reply")
            .await
            .is_err()
    );
    let original = transport.state.frames.lock().unwrap()[0].clone();
    drop(manager);
    let reopened = open(&path, transport.clone()).await;
    assert_eq!(reopened.retry_pending(CHAT_ID).await.unwrap(), 1);
    assert_eq!(
        transport.state.frames.lock().unwrap().as_slice(),
        &[original.clone(), original]
    );
}

#[tokio::test]
async fn response_transport_failure_cannot_hide_a_recovered_message_batch() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("failed-response.enc");
    let transport = Transport::new(&path);
    let manager = open(&path, transport.clone()).await;
    register(&manager).await;
    transport.state.fail.store(true, Ordering::SeqCst);
    assert!(
        manager
            .handle_incoming(CHAT_ID, &incoming(1, "second"))
            .await
            .is_err()
    );
    transport.state.fail.store(true, Ordering::SeqCst);
    let first = incoming(0, "first");
    let messages = match manager.handle_incoming(CHAT_ID, &first).await {
        Ok(messages) => messages,
        Err(_) => manager.handle_incoming(CHAT_ID, &first).await.unwrap(),
    };
    assert_eq!(messages.len(), 2, "response failure hid an accepted batch");
    assert_eq!(messages[0].text.as_deref(), Some("first"));
    assert_eq!(messages[1].text.as_deref(), Some("second"));
}

#[tokio::test]
async fn cancelled_response_preserves_the_recovered_batch_across_restart() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("cancelled-response.enc");
    let transport = Transport::new(&path);
    let manager = Arc::new(open(&path, transport.clone()).await);
    register(&manager).await;
    transport.state.fail.store(true, Ordering::SeqCst);
    assert!(
        manager
            .handle_incoming(CHAT_ID, &incoming(1, "second"))
            .await
            .is_err()
    );
    transport.state.entered.notified().await;
    transport.state.blocked.store(true, Ordering::SeqCst);
    let receiving = Arc::clone(&manager);
    let first = incoming(0, "first");
    let frame = first.clone();
    let task = tokio::spawn(async move { receiving.handle_incoming(CHAT_ID, &frame).await });
    tokio::time::timeout(Duration::from_secs(5), transport.state.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(manager);
    let reopened = open(&path, transport).await;
    let messages = reopened.handle_incoming(CHAT_ID, &first).await.unwrap();
    assert_eq!(messages.len(), 2, "cancellation erased a committed batch");
    assert_eq!(messages[0].text.as_deref(), Some("first"));
    assert_eq!(messages[1].text.as_deref(), Some("second"));
}

#[test]
fn synthetic_incoming_fixture_is_accepted_by_the_real_protocol() {
    let mut peer = SecretChat::new(i64::from(CHAT_ID), 92, 103, OWNER, ChatRole::Acceptor);
    peer.adopt_key(&CHAT_KEY).unwrap();
    assert_eq!(
        peer.receive_encrypted_frames(&incoming(0, "fixture"))
            .unwrap()[0]
            .text
            .as_deref(),
        Some("fixture")
    );
}

#[test]
fn transient_marker_refusals_match_between_memory_and_file_storage() {
    let directory = tempdir().unwrap();
    let memory = MemoryStorage::new(OWNER).unwrap();
    let file = FileStorage::open(directory.path().join("markers.enc"), STORE_KEY, OWNER).unwrap();
    for store in [&memory as &dyn SecretChatStore, &file] {
        for invalid in [i32::MIN, -1, 0] {
            assert!(store.mark_transient_suspended(invalid).is_err());
            assert!(!store.is_transient_suspended(invalid).unwrap());
        }
        store
            .save(
                &PersistedChatRecord::established(CHAT_ID, 92, 103, ChatRole::Acceptor, CHAT_KEY)
                    .unwrap(),
            )
            .unwrap();
        assert!(store.mark_transient_suspended(CHAT_ID).is_err());
        assert!(store.get(CHAT_ID).unwrap().is_some());
        assert!(!store.is_transient_suspended(CHAT_ID).unwrap());
        store.mark_transient_suspended(CHAT_ID + 1).unwrap();
        assert!(store.is_transient_suspended(CHAT_ID + 1).unwrap());
        assert!(store.mark_transient_suspended(CHAT_ID + 1).is_err());
    }
}

#[tokio::test]
async fn a_stale_manager_cannot_transmit_or_replace_a_newer_checkpoint() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("stale-writer.enc");
    let first_transport = Transport::new(&path);
    let first = open(&path, first_transport).await;
    register(&first).await;
    let stale_transport = Transport::new(&path);
    let stale = open(&path, stale_transport.clone()).await;
    let retained = first
        .send_text_with_id(CHAT_ID, 93, "current writer")
        .await
        .unwrap();
    assert!(
        stale
            .send_text_with_id(CHAT_ID, 94, "stale writer")
            .await
            .is_err()
    );
    assert!(stale_transport.state.frames.lock().unwrap().is_empty());
    let record = FileStorage::open(&path, STORE_KEY, OWNER)
        .unwrap()
        .get(CHAT_ID)
        .unwrap()
        .unwrap();
    assert_eq!(record.out_seq_no, 1);
    assert_eq!(record.pending_outbox.len(), 1);
    assert_eq!(record.pending_outbox[0].payload, retained);
}

#[tokio::test]
async fn authenticated_fatal_violation_stays_closed_when_tombstone_save_fails() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("fatal-save.enc");
    let transport = Transport::new(&path);
    let manager = open(&path, transport.clone()).await;
    register(&manager).await;
    let body = wrap_frame(
        &json!({"random_id": 1200, "ttl": 0, "message": "invalid authenticated counter"}),
        46,
        1,
        1,
    )
    .unwrap();
    let fatal = encrypt_frame(&CHAT_KEY, &body, 0).unwrap();
    let lock = lock_checkpoint(&path);
    assert!(manager.handle_incoming(CHAT_ID, &fatal).await.is_err());
    drop(lock);
    assert!(
        manager
            .send_text_with_id(CHAT_ID, 95, "must stay closed")
            .await
            .is_err()
    );
    assert!(transport.state.frames.lock().unwrap().is_empty());
    manager.close_and_tombstone(CHAT_ID, false).await.unwrap();
    drop(manager);
    let reopened = open(&path, transport).await;
    assert!(reopened.snapshot_chat(CHAT_ID).await.unwrap().tombstoned);
}

struct VolatileTransport;

impl SecretTransport for VolatileTransport {
    fn auth_identity(&self) -> &str {
        OWNER
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        _: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
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

#[tokio::test]
async fn transient_outgoing_text_uses_the_latest_authenticated_conversation_ttl() {
    let manager = TransientSecretChatManager::open_after_restart(
        VolatileTransport,
        MemoryStorage::new(OWNER).unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap();
    manager
        .register_chat(
            PersistedChatRecord::established(CHAT_ID, 92, 103, ChatRole::Acceptor, CHAT_KEY)
                .unwrap(),
        )
        .await
        .unwrap();
    let mut peer = SecretChat::new(
        i64::from(CHAT_ID),
        92,
        103,
        "synthetic-peer",
        ChatRole::Initiator,
    );
    peer.adopt_key(&CHAT_KEY).unwrap();
    for ttl in [17, 0, i32::MAX] {
        let service = peer
            .prepare_service_action(&json!({"id": 0xa1733aec_u32, "fields": {"ttl_seconds": ttl}}))
            .unwrap();
        manager
            .handle_incoming(CHAT_ID, &service.frame)
            .await
            .unwrap();
        let outgoing = manager
            .send_text(CHAT_ID, "current conversation TTL")
            .await
            .unwrap();
        let body = decrypt_frame(&CHAT_KEY, &outgoing, 8).unwrap();
        let decoded = kiaro_secret_chat::ported::schema_secret_tl::decode_object(
            hex::encode(body),
            "DecryptedMessageLayer".into(),
            0,
        )
        .unwrap();
        assert_eq!(decoded["object"]["fields"]["message"]["fields"]["ttl"], ttl);
        peer.receive_encrypted_frames(&outgoing).unwrap();
    }
}
