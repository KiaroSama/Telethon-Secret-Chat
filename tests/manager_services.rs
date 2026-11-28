use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{
    BoxFuture, CoreSecretChatManager, ManagerLimits, SecretEvent, SecretTransport,
};
use kiaro_secret_chat::protocol::{ChatRole, ChatState, SecretChat};
use serde_json::json;
use tokio::sync::Mutex;

const KEY: [u8; 256] = [0x73; 256];

#[derive(Default)]
struct CaptureTransport {
    sent: Mutex<Vec<Vec<u8>>>,
    services: AtomicUsize,
    fail_next: AtomicBool,
}

impl SecretTransport for CaptureTransport {
    fn auth_identity(&self) -> &str {
        "manager-services-audit"
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.sent.lock().await.push(frame);
            if self.fail_next.swap(false, Ordering::SeqCst) {
                Err(SecretChatError::ManagerStopping)
            } else {
                Ok(())
            }
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        self.services.fetch_add(1, Ordering::SeqCst);
        self.send_encrypted(chat_id, random_id, frame)
    }

    fn discard_encryption<'a>(
        &'a self,
        _: i64,
        _: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async {
            Err(SecretChatError::ParameterRejected {
                chat_id: None,
                reason: "credential-sentinel-must-not-leak".into(),
            })
        })
    }
}

async fn manager(
    role: ChatRole,
) -> (
    Arc<CaptureTransport>,
    CoreSecretChatManager<CaptureTransport>,
) {
    let transport = Arc::new(CaptureTransport::default());
    let manager = CoreSecretChatManager::new(transport.clone());
    manager
        .register_established_chat(7, 8, 9, role, &KEY)
        .await
        .unwrap();
    (transport, manager)
}

#[tokio::test]
async fn receiving_a_gap_sends_one_recovery_request() {
    let (_, alice) = manager(ChatRole::Initiator).await;
    let (transport, bob) = manager(ChatRole::Acceptor).await;
    alice.send_text(7, 1, "missing", 0).await.unwrap();
    let later = alice.send_text(7, 2, "later", 0).await.unwrap();
    assert!(bob.receive_frames(7, &later).await.unwrap().is_empty());
    assert_eq!(
        transport.sent.lock().await.len(),
        1,
        "the runtime never sent its Resend action"
    );
    assert!(bob.receive_frames(7, &later).await.unwrap().is_empty());
    assert_eq!(
        transport.sent.lock().await.len(),
        1,
        "a duplicate gap sent another request"
    );
}

#[tokio::test]
async fn authenticated_resend_replays_exact_retained_ciphertext() {
    let (transport, alice) = manager(ChatRole::Initiator).await;
    let original = alice.send_text(7, 10, "retained", 0).await.unwrap();
    let mut bob = SecretChat::new(7, 8, 9, "bob", ChatRole::Acceptor);
    bob.adopt_key(&KEY).unwrap();
    let request = bob
        .prepare_service_action(
            &json!({"id":0x511110b0_u32,"fields":{"start_seq_no":1,"end_seq_no":1}}),
        )
        .unwrap();
    alice.receive_frames(7, &request.frame).await.unwrap();
    let sent = transport.sent.lock().await;
    assert_eq!(
        sent.len(),
        2,
        "the manager ignored the authenticated Resend action"
    );
    assert_eq!(
        sent[1], original,
        "recovery re-encrypted the retained frame"
    );
}

#[tokio::test]
async fn transport_discard_errors_do_not_expose_adapter_credentials() {
    let (_, alice) = manager(ChatRole::Initiator).await;
    let error = alice.close_chat(7, false, "test").await.unwrap_err();
    assert!(!format!("{error:?} {error}").contains("credential-sentinel"));
}

fn dh_prime() -> Vec<u8> {
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
async fn managers_complete_rekey_with_service_rpcs_and_exchange_new_key_text() {
    let (at, alice) = manager(ChatRole::Initiator).await;
    let (bt, bob) = manager(ChatRole::Acceptor).await;
    for side in [&alice, &bob] {
        side.configure_chat_dh(7, 2, &dh_prime()).await.unwrap();
    }
    alice.start_rekey(7).await.unwrap();
    let request = at.sent.lock().await[0].clone();
    bob.receive_frames(7, &request).await.unwrap();
    let accept = bt.sent.lock().await[0].clone();
    alice.receive_frames(7, &accept).await.unwrap();
    let commit = at.sent.lock().await[1].clone();
    bob.receive_frames(7, &commit).await.unwrap();
    let noop = bt.sent.lock().await[1].clone();
    alice.receive_frames(7, &noop).await.unwrap();
    assert_eq!(&request[..8], &commit[..8]);
    assert_ne!(&request[..8], &noop[..8]);
    assert_eq!(at.services.load(Ordering::SeqCst), 2);
    assert_eq!(bt.services.load(Ordering::SeqCst), 2);
    let frame = alice.send_text(7, 71, "new key", 0).await.unwrap();
    assert_eq!(
        bob.receive_frames(7, &frame).await.unwrap()[0]
            .text
            .as_deref(),
        Some("new key")
    );
    let events = alice.drain_events().await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SecretEvent::ServiceActionReceived { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SecretEvent::MessageReceived { .. }))
    );
}

#[tokio::test]
async fn service_failure_retains_exact_frame_and_sequence_for_retry() {
    let (transport, alice) = manager(ChatRole::Initiator).await;
    transport.fail_next.store(true, Ordering::SeqCst);
    assert!(matches!(
        alice
            .send_service_action(7, &json!({"id":0xa82fdd63_u32,"fields":{}}))
            .await,
        Err(SecretChatError::SendPending { .. })
    ));
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 1);
    assert_eq!(alice.retry_pending(7).await.unwrap(), 1);
    let sent = transport.sent.lock().await;
    assert_eq!(sent[0], sent[1]);
    assert_eq!(transport.services.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn service_capacity_refusal_does_not_consume_sequence_or_exchange() {
    let transport = Arc::new(CaptureTransport::default());
    let alice = CoreSecretChatManager::with_limits(
        transport,
        ManagerLimits {
            max_retained_messages: 1,
            max_retained_bytes: 4096,
        },
    )
    .unwrap();
    alice
        .register_established_chat(7, 8, 9, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    alice.configure_chat_dh(7, 2, &dh_prime()).await.unwrap();
    let first = alice.send_text(7, 1, "retained", 0).await.unwrap();
    assert!(alice.start_rekey(7).await.is_err());
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 1);
    let (_, bob) = manager(ChatRole::Acceptor).await;
    let ack = bob
        .send_text(7, 2, "ack cannot jump unseen message", 0)
        .await
        .unwrap();
    alice.receive_frames(7, &ack).await.unwrap();
    assert!(
        alice.start_rekey(7).await.is_err(),
        "unacknowledged retention was unexpectedly lost"
    );
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 1);
    bob.receive_frames(7, &first).await.unwrap();
    let ack = bob.send_text(7, 3, "ack after receiving", 0).await.unwrap();
    alice.receive_frames(7, &ack).await.unwrap();
    assert!(
        alice.start_rekey(7).await.unwrap(),
        "refused staging left a phantom exchange active"
    );
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 2);
}

#[tokio::test]
async fn unsatisfiable_resend_closes_chat_instead_of_ignoring_the_request() {
    let (_, alice) = manager(ChatRole::Initiator).await;
    let mut bob = SecretChat::new(7, 8, 9, "bob", ChatRole::Acceptor);
    bob.adopt_key(&KEY).unwrap();
    let frame = bob
        .prepare_service_action(
            &json!({"id":0x511110b0_u32,"fields":{"start_seq_no":1,"end_seq_no":1}}),
        )
        .unwrap();
    assert!(matches!(
        alice.receive_frames(7, &frame.frame).await,
        Err(SecretChatError::ResendUnsatisfiable { .. })
    ));
    assert_eq!(alice.chat_state(7).await.unwrap(), ChatState::Closed);
}

#[tokio::test]
async fn missing_rekey_parameters_fail_closed() {
    let (_, alice) = manager(ChatRole::Initiator).await;
    assert!(alice.start_rekey(7).await.is_err());
    assert_eq!(alice.chat_state(7).await.unwrap(), ChatState::Closed);
}

#[tokio::test]
async fn receive_capacity_failure_leaves_gap_and_sequence_retryable() {
    let at = Arc::new(CaptureTransport::default());
    let alice = CoreSecretChatManager::with_limits(
        at.clone(),
        ManagerLimits {
            max_retained_messages: 1,
            max_retained_bytes: 4096,
        },
    )
    .unwrap();
    alice
        .register_established_chat(7, 8, 9, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    let (_, bob) = manager(ChatRole::Acceptor).await;
    let outbound = alice.send_text(7, 1, "awaiting ack", 0).await.unwrap();
    bob.receive_frames(7, &outbound).await.unwrap();
    let mut frames = Vec::new();
    for id in 10..13 {
        frames.push(bob.send_text(7, id, "response", 0).await.unwrap());
    }
    assert!(alice.receive_frames(7, &frames[2]).await.is_err());
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 1);
    assert_eq!(at.services.load(Ordering::SeqCst), 0);
    assert_eq!(alice.receive_frames(7, &frames[0]).await.unwrap().len(), 1);
    assert!(
        alice
            .receive_frames(7, &frames[2])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(at.services.load(Ordering::SeqCst), 1);
    assert_eq!(alice.receive_frames(7, &frames[1]).await.unwrap().len(), 2);
}

#[tokio::test]
async fn automatic_rekey_is_queued_at_the_message_threshold() {
    let (transport, alice) = manager(ChatRole::Initiator).await;
    alice.configure_chat_dh(7, 2, &dh_prime()).await.unwrap();
    for id in 0..100 {
        alice.send_text(7, id, "threshold", 0).await.unwrap();
    }
    assert_eq!(transport.services.load(Ordering::SeqCst), 0);
    alice.send_text(7, 100, "trigger", 0).await.unwrap();
    assert_eq!(transport.services.load(Ordering::SeqCst), 1);
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 102);
}

#[tokio::test]
async fn incoming_traffic_triggers_rekey_without_another_local_send() {
    let (transport, alice) = manager(ChatRole::Initiator).await;
    alice.configure_chat_dh(7, 2, &dh_prime()).await.unwrap();
    alice.send_text(7, 1, "one outgoing", 0).await.unwrap();
    let mut bob = SecretChat::new(7, 8, 9, "bob", ChatRole::Acceptor);
    bob.adopt_key(&KEY).unwrap();
    for _ in 0..100 {
        let incoming = bob
            .prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))
            .unwrap();
        alice.receive_frames(7, &incoming.frame).await.unwrap();
    }
    assert_eq!(transport.services.load(Ordering::SeqCst), 1);
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 2);
}

#[tokio::test]
async fn oversized_text_is_rejected_without_consuming_sequence() {
    let (_, alice) = manager(ChatRole::Initiator).await;
    let text = "x".repeat(kiaro_secret_chat::protocol::MAX_ENCRYPTED_FRAME_LEN);
    assert!(alice.send_text(7, 5, &text, 0).await.is_err());
    assert_eq!(alice.out_seq_no(7).await.unwrap(), 0);
}
