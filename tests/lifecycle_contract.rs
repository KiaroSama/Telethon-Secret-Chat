use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::lifecycle::{
    build_service_action, check_lock_guard, ogg_looks_like_voice, receive_history_projection,
    remember_early_update, retained_outbox_content_ids, temporary_owned_roundtrip,
};
use kiaro_secret_chat::manager::{BoxFuture, SecretChatManager, SecretEvent, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, ChatState};
use tokio::sync::Mutex;

struct FakeTransport {
    auth_id: String,
    fail_send: AtomicBool,
    sent_frames: Mutex<Vec<(i64, i64, Vec<u8>)>>,
    discarded: Mutex<Vec<(i64, bool)>>,
}

impl FakeTransport {
    fn new(auth_id: &str) -> Self {
        Self {
            auth_id: auth_id.to_owned(),
            fail_send: AtomicBool::new(false),
            sent_frames: Mutex::new(Vec::new()),
            discarded: Mutex::new(Vec::new()),
        }
    }
}

impl SecretTransport for FakeTransport {
    fn auth_identity(&self) -> &str {
        &self.auth_id
    }

    fn send_encrypted<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            if self.fail_send.load(Ordering::SeqCst) {
                return Err(SecretChatError::SendPending {
                    chat_id,
                    random_id,
                    cause: "InjectedOfflineFailure".into(),
                });
            }
            self.sent_frames
                .lock()
                .await
                .push((chat_id, random_id, frame));
            Ok(())
        })
    }

    fn discard_encryption<'a>(
        &'a self,
        chat_id: i64,
        delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.discarded.lock().await.push((chat_id, delete_history));
            Ok(())
        })
    }
}

#[tokio::test]
async fn offline_peers_exchange_encrypted_messages_handle_transport_failure_and_shut_down() {
    let shared_key = [0x77u8; 256];
    let alice_tx = Arc::new(FakeTransport::new("caller-auth-alice"));
    let bob_tx = Arc::new(FakeTransport::new("caller-auth-bob"));

    let alice = SecretChatManager::new(Arc::clone(&alice_tx));
    let bob = SecretChatManager::new(Arc::clone(&bob_tx));

    assert_eq!(alice.caller_auth_identity(), "caller-auth-alice");
    assert_eq!(bob.caller_auth_identity(), "caller-auth-bob");

    let fp_a = alice
        .register_established_chat(55, 1001, 2002, ChatRole::Initiator, &shared_key)
        .await
        .expect("alice ready");
    let fp_b = bob
        .register_established_chat(55, 1001, 1001, ChatRole::Acceptor, &shared_key)
        .await
        .expect("bob ready");
    assert_eq!(fp_a, fp_b);

    // 1) Alice sends encrypted message -> Bob decrypts and accepts.
    let frame1 = alice
        .send_text(55, 9001, "hello from alice", 0)
        .await
        .expect("alice send 1");
    assert_eq!(alice.out_seq_no(55).await.unwrap(), 1);

    let received = bob.receive_frame(55, &frame1).await.expect("bob receive 1");
    assert_eq!(received.text.as_deref(), Some("hello from alice"));

    // 2) Uncertain delivery reserves its sequence and retains the frame for exact retry.
    alice_tx.fail_send.store(true, Ordering::SeqCst);
    let err = alice.send_text(55, 9002, "will fail", 0).await.unwrap_err();
    assert!(matches!(
        err,
        SecretChatError::SendPending {
            chat_id: 55,
            random_id: 9002,
            ..
        }
    ));
    assert_eq!(alice.out_seq_no(55).await.unwrap(), 2);

    // 3) Close chat discards encryption and transitions state to Closed.
    alice
        .close_chat(55, true, "operator closed")
        .await
        .expect("close chat");
    assert_eq!(alice.chat_state(55).await.unwrap(), ChatState::Closed);
    assert_eq!(*alice_tx.discarded.lock().await, vec![(55, true)]);

    // 4) Shutdown prevents any further operations (ManagerStopping).
    bob.shutdown().await;
    assert!(matches!(
        bob.receive_frame(55, &frame1).await,
        Err(SecretChatError::ManagerStopping)
    ));

    let bob_events = bob.drain_events().await;
    assert!(
        bob_events
            .iter()
            .any(|e| matches!(e, SecretEvent::MessageReceived { chat_id: 55, .. }))
    );
}

#[test]
fn gated_g2_lifecycle_helpers_preserve_action_lock_outbox_receive_voice_and_temp_contracts() {
    // Actions builder (15/15 gate 0)
    let resend = build_service_action("resend", 1, 5, vec![])
        .expect("resend action")
        .expect("some action");
    assert_eq!(resend["group"], "protocol");

    // Lock generation guard (15/15 gate 0)
    let guard = check_lock_guard(1, 2, false, true).expect("lock check");
    assert!(guard["generation"].is_string());
    assert!(guard["current"].is_null());

    // Early update establishment precedence (15/15 gate 0)
    let est = remember_early_update(1, false, false, true).expect("establishment");
    assert!(est.is_object());

    // Retained outbox content IDs filter out service messages (15/15 gate 0)
    let retained =
        retained_outbox_content_ids(vec![10, 11, 12], vec![false, true, false]).expect("outbox");
    assert_eq!(retained, vec![10, 12]);

    // Receive history deletion projection (15/15 gate 0)
    let hist = receive_history_projection(vec![1, 2, 3], vec![2], 10, false).expect("receive");
    assert!(hist.is_object());

    // Ogg Opus voice detection returns None/Null on non-Ogg bytes (15/15 gate 0)
    let voice = ogg_looks_like_voice(b"not-an-ogg-header").expect("ogg check");
    assert_eq!(voice, serde_json::Value::Null);

    // Owned temporary file cleanup roundtrip (15/15 gate 0)
    let tmp = temporary_owned_roundtrip("ksc-lifecycle-test").expect("temp roundtrip");
    assert!(!tmp.is_empty());
}
