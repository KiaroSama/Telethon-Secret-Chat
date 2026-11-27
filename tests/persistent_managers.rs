#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kiaro_secret_chat::custody::{CustodyLimits, CustodyStore};
use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::ingress::{
    DurableIngressController, EncryptedIngressStore, IngressLimits, IngressOrigin, IngressUpdate,
};
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, ChatState};
use kiaro_secret_chat::runtime_modes::{
    BoundedSecretChatManager, PersistentSecretChatManager, StrongSecretChatManager,
    TransientSecretChatManager,
};
use kiaro_secret_chat::storage_backend::{
    FileStorage, MemoryStorage, PersistedChatRecord, PersistedStoreSnapshot, ProtectedFileStorage,
    SecretChatStore,
};
use tempfile::tempdir;

const ID: i32 = 7;
const ALICE: &str = "alice-test-authorization";
const BOB: &str = "bob-test-authorization";
const KEY: [u8; 32] = [0x4c; 32];
const CHAT_KEY: [u8; 256] = [0x42; 256];

#[derive(Clone)]
struct Transport {
    owner: &'static str,
    frames: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl Transport {
    fn new(owner: &'static str) -> Self {
        Self {
            owner,
            frames: Arc::default(),
        }
    }

    fn take_frame(&self) -> Vec<u8> {
        self.frames.lock().unwrap().pop_front().unwrap()
    }
}

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        self.owner
    }

    fn send_encrypted<'a>(
        &'a self,
        _chat_id: i64,
        _random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.frames.lock().unwrap().push_back(frame);
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

    fn discard_encryption<'a>(
        &'a self,
        _chat_id: i64,
        _delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

fn chat(role: ChatRole) -> PersistedChatRecord {
    PersistedChatRecord::established(ID, 101, 202, role, CHAT_KEY).unwrap()
}

async fn transfer<S: SecretChatStore>(
    from: &PersistentSecretChatManager<Transport, S>,
    transport: &Transport,
    to: &PersistentSecretChatManager<Transport, S>,
    text: &str,
) {
    let sent = from.send_text(ID, text).await.unwrap();
    let on_wire = transport.take_frame();
    assert_eq!(sent, on_wire);
    let accepted = to.handle_incoming(ID, &on_wire).await.unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].text.as_deref(), Some(text));
}

#[tokio::test]
async fn encrypted_restart_preserves_state_and_continues_both_directions() {
    let dir = tempdir().unwrap();
    let alice_path = dir.path().join("alice state");
    let bob_path = dir.path().join("bob state");
    let alice_transport = Transport::new(ALICE);
    let bob_transport = Transport::new(BOB);
    let open = |transport: Transport, path: &std::path::Path| {
        let store = FileStorage::open(path, KEY, transport.owner).unwrap();
        PersistentSecretChatManager::open(transport, store, ManagerLimits::default())
    };
    let alice = open(alice_transport.clone(), &alice_path).await.unwrap();
    let bob = open(bob_transport.clone(), &bob_path).await.unwrap();
    alice
        .register_and_persist(chat(ChatRole::Initiator))
        .await
        .unwrap();
    bob.register_and_persist(chat(ChatRole::Acceptor))
        .await
        .unwrap();
    alice.set_ttl(ID, 13).await.unwrap();
    bob.handle_incoming(ID, &alice_transport.take_frame())
        .await
        .unwrap();
    transfer(&alice, &alice_transport, &bob, "message one").await;
    transfer(&bob, &bob_transport, &alice, "message two").await;
    transfer(&alice, &alice_transport, &bob, "message three").await;
    let alice_before = alice.snapshot_chat(ID).await.unwrap();
    let bob_before = bob.snapshot_chat(ID).await.unwrap();
    assert_eq!((alice_before.in_seq_no, alice_before.out_seq_no), (1, 3));
    assert_eq!((bob_before.in_seq_no, bob_before.out_seq_no), (3, 1));
    drop((alice, bob));

    let alice = open(alice_transport.clone(), &alice_path).await.unwrap();
    let bob = open(bob_transport.clone(), &bob_path).await.unwrap();
    for (before, after) in [
        (alice_before, alice.snapshot_chat(ID).await.unwrap()),
        (bob_before, bob.snapshot_chat(ID).await.unwrap()),
    ] {
        assert_eq!(before.in_seq_no, after.in_seq_no);
        assert_eq!(before.out_seq_no, after.out_seq_no);
        assert_eq!(before.layer, after.layer);
        assert_eq!(after.ttl, 13);
        assert_eq!(before.key_fingerprint, after.key_fingerprint);
        assert_eq!(before.sent_since_rekey, after.sent_since_rekey);
        assert_eq!(before.received_since_rekey, after.received_since_rekey);
        assert_eq!(before.pending_outbox, after.pending_outbox);
    }
    transfer(&bob, &bob_transport, &alice, "after restart").await;
    transfer(&alice, &alice_transport, &bob, "return after restart").await;
    alice.close_and_tombstone(ID, false).await.unwrap();
    drop(alice);
    let reopened = open(alice_transport, &alice_path).await.unwrap();
    assert!(reopened.list_persisted_chats().await.unwrap().is_empty());
    let tombstone = reopened.snapshot_chat(ID).await.unwrap();
    assert!(tombstone.tombstoned);
    assert_eq!(tombstone.state, ChatState::Closed);
    assert_eq!(tombstone.auth_key, [0; 256]);
}

#[test]
fn authenticated_files_reject_wrong_key_wrong_owner_and_dirty_lock() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("chat.store");
    let store = ProtectedFileStorage::open(&path, KEY, ALICE).unwrap();
    store.save(&chat(ChatRole::Initiator)).unwrap();
    drop(store);
    assert!(ProtectedFileStorage::open(&path, [0x11; 32], ALICE).is_err());
    assert!(ProtectedFileStorage::open(&path, KEY, BOB).is_err());
    assert!(ProtectedFileStorage::open(&path, KEY, ALICE).is_ok());
    let path = dir.path().join("plain.store");
    let store = FileStorage::open(&path, KEY, ALICE).unwrap();
    store.save(&chat(ChatRole::Initiator)).unwrap();
    drop(store);
    assert!(FileStorage::open(&path, [0x11; 32], ALICE).is_err());
    let lock_path = dir.path().join("plain.store.lock");
    std::fs::write(&lock_path, b"not empty").unwrap();
    assert!(FileStorage::open(&path, KEY, ALICE).is_err());
    std::fs::write(lock_path, []).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(FileStorage::open(&path, KEY, ALICE).is_err());
    assert!(FileStorage::open("relative.store", KEY, ALICE).is_err());
}

#[test]
fn the_storage_schema_rejects_duplicates_unknown_fields_and_future_versions() {
    let record = serde_json::to_string(&chat(ChatRole::Initiator)).unwrap();
    let parse = |chats: &str, suspended: &str, version: u32| {
        serde_json::from_str::<PersistedStoreSnapshot>(&format!(
            "{{\"format_version\":{version},\"account_binding\":\"fixture\",\"chats\":{chats},\"suspended_transient_chats\":{suspended}}}"
        ))
    };
    assert!(parse(&format!("{{\"7\":{record}}}"), "[]", 1).is_ok());
    assert!(parse(&format!("{{\"7\":{record},\"7\":{record}}}"), "[]", 1).is_err());
    assert!(parse("{}", "[1,1]", 1).is_err());
    assert!(parse("{}", "[]", 2).is_err());
    assert!(
        serde_json::from_str::<PersistedChatRecord>(&record.replacen('{', "{\"unexpected\":0,", 1))
            .is_err()
    );
}

#[tokio::test]
async fn bounded_pause_survives_a_sweep_and_missed_ingress_blocks_resume() {
    let dir = tempdir().unwrap();
    let durable = PersistentSecretChatManager::open(
        Transport::new(ALICE),
        MemoryStorage::new(ALICE).unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap();
    let bounded = BoundedSecretChatManager::new(
        durable,
        CustodyStore::open_durable(dir.path().join("custody"), KEY).unwrap(),
        CustodyLimits {
            max_retained_messages: 1,
            max_stored_bytes: 2048,
        },
        Duration::ZERO,
    );
    bounded
        .register_and_persist(chat(ChatRole::Initiator))
        .await
        .unwrap();
    assert_eq!(bounded.sweep_custody(i64::MAX).await.unwrap(), vec![ID]);
    assert!(bounded.custody_status(ID).await.unwrap().is_paused());
    assert!(bounded.send_text(ID, "must not send").await.is_err());
    assert!(bounded.sweep_custody(i64::MAX).await.unwrap().is_empty());
    bounded.resume_chat(ID).await.unwrap();
    assert!(!bounded.custody_status(ID).await.unwrap().is_paused());
    bounded.pause_chat(ID).await.unwrap();
    bounded.note_missed_ingress(ID).await.unwrap();
    assert!(bounded.resume_chat(ID).await.is_err());
}

#[tokio::test]
async fn transient_store_contains_only_a_restart_marker() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("markers");
    let transport = Transport::new(ALICE);
    let open = || {
        TransientSecretChatManager::open_after_restart(
            transport.clone(),
            FileStorage::open(&path, KEY, ALICE).unwrap(),
            ManagerLimits::default(),
        )
    };
    let transient = open().await.unwrap();
    transient
        .register_chat(chat(ChatRole::Initiator))
        .await
        .unwrap();
    let durable_before = std::fs::read(&path).unwrap();
    transient
        .send_text(ID, "this payload is kept only in memory")
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), durable_before);
    drop(transient);
    {
        let store = FileStorage::open(&path, KEY, ALICE).unwrap();
        assert!(store.get(ID).unwrap().is_none());
        assert!(store.is_transient_suspended(ID).unwrap());
    }
    let transient = open().await.unwrap();
    let suspended = SecretChatError::Protocol("transient chat permanently suspended after restart");
    assert_eq!(
        transient.send_text(ID, "blocked").await.unwrap_err(),
        suspended
    );
    assert_eq!(
        transient.handle_incoming(ID, &[]).await.unwrap_err(),
        suspended
    );
    assert_eq!(
        transient
            .register_chat(chat(ChatRole::Initiator))
            .await
            .unwrap_err(),
        suspended
    );
}

#[tokio::test]
async fn strong_ingress_commits_before_return_and_consumers_settle_contiguously() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ingress");
    let store_path = dir.path().join("chat state");
    let open = || {
        let store = FileStorage::open(&store_path, KEY, ALICE).unwrap();
        let path = path.clone();
        async move {
            let persistent = PersistentSecretChatManager::open(
                Transport::new(ALICE),
                store,
                ManagerLimits::default(),
            )
            .await
            .unwrap();
            let ingress = DurableIngressController::restore(
                ALICE,
                EncryptedIngressStore::open(path, KEY).unwrap(),
                IngressLimits::default(),
            )
            .unwrap();
            StrongSecretChatManager::new(persistent, ingress).unwrap()
        }
    };
    let mode = open().await;
    mode.register_and_persist(chat(ChatRole::Acceptor))
        .await
        .unwrap();
    let update = |order, provenance| IngressUpdate {
        stream_id: StrongSecretChatManager::<Transport, FileStorage>::stream_id(ID),
        order,
        provenance,
        associated_chat_id: Some(i64::from(ID)),
        payload: vec![u8::try_from(order).unwrap()],
    };
    assert!(
        mode.ingest_before_ack(update(1, IngressOrigin::LiveSocket))
            .await
            .is_err()
    );
    for order in [2, 1, 3] {
        mode.ingest_before_ack(update(order, IngressOrigin::VerifiedTransport))
            .await
            .unwrap();
    }
    assert!(path.is_file());
    assert_eq!(mode.applied_cursor(ID).await.unwrap(), 0);
    assert!(mode.acknowledge_delivery(ID, 2).await.is_err());
    let page = mode.list_pending_deliveries(ID, 1, 1).await.unwrap();
    assert_eq!(page.iter().map(|r| r.order).collect::<Vec<_>>(), vec![2]);
    assert_eq!(
        mode.list_pending_deliveries(ID, 10, 0).await.unwrap().len(),
        3
    );
    drop(mode);

    let mode = open().await;
    assert_eq!(
        mode.list_pending_deliveries(ID, 10, 0).await.unwrap().len(),
        3
    );
    assert!(mode.acknowledge_delivery(ID, 1).await.unwrap());
    assert_eq!(mode.applied_cursor(ID).await.unwrap(), 1);
    assert!(!mode.acknowledge_delivery(ID, 1).await.unwrap());
    drop(mode);
    let mode = open().await;
    assert_eq!(mode.applied_cursor(ID).await.unwrap(), 1);
    assert_eq!(
        mode.list_pending_deliveries(ID, 10, 0)
            .await
            .unwrap()
            .iter()
            .map(|r| r.order)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[tokio::test]
async fn strong_replay_isolates_closed_and_non_ready_chats_without_poisoning_ingress() {
    let dir = tempdir().unwrap();
    let alice_state = dir.path().join("alice-replay.bin");
    let bob_state = dir.path().join("bob-replay.bin");
    let bob_ingress = dir.path().join("bob-ingress.bin");

    let alice_t = Transport::new(ALICE);
    let bob_t = Transport::new(BOB);

    let alice_store = FileStorage::open(&alice_state, KEY, ALICE).unwrap();
    let alice =
        PersistentSecretChatManager::open(alice_t.clone(), alice_store, ManagerLimits::default())
            .await
            .unwrap();

    // Register two chats: closed_id (41) will be closed AFTER ingress captures a frame,
    // while live_id (42) stays healthy and ready across restart.
    let closed_id = 41;
    let live_id = 42;
    let make_chat = |id: i32, role: ChatRole| {
        PersistedChatRecord::established(id, 901 + i64::from(id), 99, role, [0x42; 256]).unwrap()
    };
    alice
        .register_and_persist(make_chat(closed_id, ChatRole::Initiator))
        .await
        .unwrap();
    alice
        .register_and_persist(make_chat(live_id, ChatRole::Initiator))
        .await
        .unwrap();
    let closed_frame = alice
        .send_text(closed_id, "stale frame for closed chat")
        .await
        .unwrap();
    let _ = alice_t.take_frame();
    let live_frame = alice
        .send_text(live_id, "healthy frame survives")
        .await
        .unwrap();
    let _ = alice_t.take_frame();

    // Initial Bob startup: register both chats and capture durable ingress for both.
    let open_bob = |t: Transport| {
        let bob_state = bob_state.clone();
        let bob_ingress = bob_ingress.clone();
        async move {
            let store = FileStorage::open(&bob_state, KEY, BOB).unwrap();
            let persistent = PersistentSecretChatManager::open(t, store, ManagerLimits::default())
                .await
                .unwrap();
            let ing_store = EncryptedIngressStore::open(&bob_ingress, [0x71; 32]).unwrap();
            let ingress = if bob_ingress.exists() {
                DurableIngressController::restore(BOB, ing_store, IngressLimits::default()).unwrap()
            } else {
                DurableIngressController::new(BOB, ing_store, IngressLimits::default())
            };
            StrongSecretChatManager::new(persistent, ingress).unwrap()
        }
    };
    let bob = open_bob(bob_t.clone()).await;
    bob.register_and_persist(make_chat(closed_id, ChatRole::Acceptor))
        .await
        .unwrap();
    bob.register_and_persist(make_chat(live_id, ChatRole::Acceptor))
        .await
        .unwrap();
    bob.ingest_before_ack(IngressUpdate {
        stream_id: StrongSecretChatManager::<Transport, FileStorage>::stream_id(closed_id),
        order: 1,
        provenance: IngressOrigin::VerifiedTransport,
        associated_chat_id: Some(i64::from(closed_id)),
        payload: closed_frame,
    })
    .await
    .unwrap();
    bob.ingest_before_ack(IngressUpdate {
        stream_id: StrongSecretChatManager::<Transport, FileStorage>::stream_id(live_id),
        order: 1,
        provenance: IngressOrigin::VerifiedTransport,
        associated_chat_id: Some(i64::from(live_id)),
        payload: live_frame,
    })
    .await
    .unwrap();
    drop(bob);

    // Simulate out-of-band chat closure/tombstone leaving stale pending ingress behind
    // (the exact failure mode that previously crashed startup replay with Error::ChatState).
    let store = FileStorage::open(&bob_state, KEY, BOB).unwrap();
    let persistent_only =
        PersistentSecretChatManager::open(bob_t.clone(), store, ManagerLimits::default())
            .await
            .unwrap();
    persistent_only
        .close_and_tombstone(closed_id, true)
        .await
        .unwrap();
    drop(persistent_only);

    // Reopen StrongSecretChatManager and run startup replay: must succeed cleanly,
    // discard closed_id's stale ingress, apply live_id's message, and keep ingress healthy.
    let bob = open_bob(bob_t.clone()).await;
    let outcome = bob.replay_pending().await.unwrap();
    assert_eq!(outcome.discarded_closed, vec![(closed_id, 1)]);
    assert!(outcome.chat_errors.is_empty());
    assert_eq!(outcome.applied.len(), 1);
    assert_eq!(outcome.applied[0].0, live_id);
    assert_eq!(outcome.applied[0].1, 1);
    assert_eq!(
        outcome.applied[0].2[0].text.as_deref(),
        Some("healthy frame survives")
    );
    assert!(bob.acknowledge_delivery(live_id, 1).await.unwrap());
    assert_eq!(bob.applied_cursor(live_id).await.unwrap(), 1);

    // Subsequent startup replay / session health check stays clean with zero IngressRefused.
    let second = bob.replay_pending().await.unwrap();
    assert!(second.applied.is_empty());
    assert!(second.discarded_closed.is_empty());
    assert!(second.chat_errors.is_empty());
}
