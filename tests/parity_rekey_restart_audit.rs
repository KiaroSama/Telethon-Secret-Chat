#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretTransport};
use kiaro_secret_chat::protocol::{AcceptedMessage, ChatRole, KeyLifecycleState};
use kiaro_secret_chat::runtime_modes::PersistentSecretChatManager;
use kiaro_secret_chat::storage_backend::{FileStorage, PersistedChatRecord};
use tempfile::tempdir;

const CHAT_ID: i32 = 57;
const STORE_KEY: [u8; 32] = [0x28; 32];
const INITIAL_KEY: [u8; 256] = [0x4d; 256];
const REQUEST: u32 = 0xf3c9611b;
const ACCEPT: u32 = 0x6fe1735b;
const COMMIT: u32 = 0xec2e0b9b;
const NOOP: u32 = 0xa82fdd63;

type SentFrames = Arc<Mutex<VecDeque<(bool, Vec<u8>)>>>;

#[derive(Clone)]
struct Transport {
    owner: &'static str,
    sent: SentFrames,
}

impl Transport {
    fn new(owner: &'static str) -> Self {
        Self {
            owner,
            sent: Arc::default(),
        }
    }

    fn take(&self, service: bool) -> Vec<u8> {
        let (actual_service, frame) = self.sent.lock().unwrap().pop_front().unwrap();
        assert_eq!(actual_service, service);
        frame
    }
}

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        self.owner
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push_back((false, frame));
            Ok(())
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push_back((true, frame));
            Ok(())
        })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

type Manager = PersistentSecretChatManager<Transport, FileStorage>;

async fn open(path: &Path, transport: &Transport) -> Manager {
    PersistentSecretChatManager::open(
        transport.clone(),
        FileStorage::open(path, STORE_KEY, transport.owner).unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap()
}

async fn reopen(manager: Manager, path: &Path, transport: &Transport, stage: &str) -> Manager {
    let before = manager.snapshot_chat(CHAT_ID).await.unwrap();
    drop(manager);
    let manager = open(path, transport).await;
    let after = manager.snapshot_chat(CHAT_ID).await.unwrap();
    assert_eq!(before.rekey_state, after.rekey_state, "{stage}");
    assert_eq!(before.key_fingerprint, after.key_fingerprint, "{stage}");
    assert_eq!(
        (before.in_seq_no, before.out_seq_no),
        (after.in_seq_no, after.out_seq_no),
        "{stage}"
    );
    assert_eq!(before.pending_outbox, after.pending_outbox, "{stage}");
    // Compare the complete stored protocol without printing synthetic private exponents.
    assert!(
        serde_json::to_value(&before).unwrap() == serde_json::to_value(&after).unwrap(),
        "the {stage} checkpoint changed while reopening"
    );
    manager
}

fn assert_action(messages: &[AcceptedMessage], action: u32) {
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].action.as_ref().unwrap()["id"], action);
}

fn dh_prime() -> Vec<u8> {
    // RFC 3526 group 14, shared with the existing real-DH manager acceptance test.
    hex::decode(concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
        "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
        "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
        "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    ))
    .unwrap()
}

#[tokio::test]
async fn persistent_rekey_reopens_each_exchange_boundary_and_a_valid_message_gap() {
    let directory = tempdir().unwrap();
    let alice_path = directory.path().join("alice.enc");
    let bob_path = directory.path().join("bob.enc");
    let alice_transport = Transport::new("rekey-restart-alice");
    let bob_transport = Transport::new("rekey-restart-bob");
    let mut alice = open(&alice_path, &alice_transport).await;
    let mut bob = open(&bob_path, &bob_transport).await;
    for (manager, role) in [(&alice, ChatRole::Initiator), (&bob, ChatRole::Acceptor)] {
        manager
            .register_and_persist(
                PersistedChatRecord::established(CHAT_ID, 58, 59, role, INITIAL_KEY).unwrap(),
            )
            .await
            .unwrap();
        manager
            .configure_chat_dh(CHAT_ID, 2, &dh_prime())
            .await
            .unwrap();
    }
    let old_fingerprint = alice.snapshot_chat(CHAT_ID).await.unwrap().key_fingerprint;

    assert!(alice.start_rekey(CHAT_ID).await.unwrap());
    let request = alice_transport.take(true);
    assert!(matches!(
        alice.snapshot_chat(CHAT_ID).await.unwrap().rekey_state,
        KeyLifecycleState::Rekeying { .. }
    ));
    alice = reopen(alice, &alice_path, &alice_transport, "Request").await;
    bob = reopen(bob, &bob_path, &bob_transport, "before Request receipt").await;

    assert_action(
        &bob.handle_incoming(CHAT_ID, &request).await.unwrap(),
        REQUEST,
    );
    let accept = bob_transport.take(true);
    assert!(matches!(
        bob.snapshot_chat(CHAT_ID).await.unwrap().rekey_state,
        KeyLifecycleState::Rekeying { .. }
    ));
    alice = reopen(
        alice,
        &alice_path,
        &alice_transport,
        "before Accepted receipt",
    )
    .await;
    bob = reopen(bob, &bob_path, &bob_transport, "Accepted").await;

    assert_action(
        &alice.handle_incoming(CHAT_ID, &accept).await.unwrap(),
        ACCEPT,
    );
    let commit = alice_transport.take(true);
    let switched = alice.snapshot_chat(CHAT_ID).await.unwrap();
    let new_fingerprint = switched.key_fingerprint;
    assert_ne!(new_fingerprint, old_fingerprint);
    assert!(switched.previous_auth_key.is_some());
    assert_eq!(&commit[..8], &old_fingerprint.to_le_bytes());
    alice = reopen(alice, &alice_path, &alice_transport, "Commit").await;
    bob = reopen(bob, &bob_path, &bob_transport, "before Commit receipt").await;

    assert_action(
        &bob.handle_incoming(CHAT_ID, &commit).await.unwrap(),
        COMMIT,
    );
    let noop = bob_transport.take(true);
    assert_eq!(&noop[..8], &new_fingerprint.to_le_bytes());
    assert_eq!(
        bob.snapshot_chat(CHAT_ID).await.unwrap().key_fingerprint,
        new_fingerprint
    );
    alice = reopen(alice, &alice_path, &alice_transport, "before Noop receipt").await;
    bob = reopen(bob, &bob_path, &bob_transport, "Noop").await;

    assert_action(&alice.handle_incoming(CHAT_ID, &noop).await.unwrap(), NOOP);
    assert!(
        alice
            .snapshot_chat(CHAT_ID)
            .await
            .unwrap()
            .previous_auth_key
            .is_none()
    );
    alice = reopen(alice, &alice_path, &alice_transport, "confirmed exchange").await;
    bob = reopen(bob, &bob_path, &bob_transport, "confirmed peer").await;

    let outbound = alice
        .send_text_with_id(CHAT_ID, 201, "new key from Alice")
        .await
        .unwrap();
    assert_eq!(alice_transport.take(false), outbound);
    assert_eq!(&outbound[..8], &new_fingerprint.to_le_bytes());
    assert_eq!(
        bob.handle_incoming(CHAT_ID, &outbound).await.unwrap()[0]
            .text
            .as_deref(),
        Some("new key from Alice")
    );
    let response = bob
        .send_text_with_id(CHAT_ID, 202, "new key from Bob")
        .await
        .unwrap();
    assert_eq!(bob_transport.take(false), response);
    assert_eq!(&response[..8], &new_fingerprint.to_le_bytes());
    assert_eq!(
        alice.handle_incoming(CHAT_ID, &response).await.unwrap()[0]
            .text
            .as_deref(),
        Some("new key from Bob")
    );

    // A valid new-key gap must keep its plaintext queue, provenance, and resend metadata.
    let first = alice
        .send_text_with_id(CHAT_ID, 203, "gap first")
        .await
        .unwrap();
    let second = alice
        .send_text_with_id(CHAT_ID, 204, "gap second")
        .await
        .unwrap();
    assert_eq!(alice_transport.take(false), first);
    assert_eq!(alice_transport.take(false), second);
    assert!(
        bob.handle_incoming(CHAT_ID, &second)
            .await
            .unwrap()
            .is_empty()
    );
    let _resend = bob_transport.take(true);
    bob = reopen(bob, &bob_path, &bob_transport, "valid pending gap").await;
    let recovered = bob.handle_incoming(CHAT_ID, &first).await.unwrap();
    assert_eq!(
        recovered
            .iter()
            .map(|message| message.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("gap first"), Some("gap second")]
    );
    assert!(
        bob.handle_incoming(CHAT_ID, &second)
            .await
            .unwrap()
            .is_empty()
    );
}
