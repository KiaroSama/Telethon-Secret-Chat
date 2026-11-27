use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{
    BoxFuture, ManagerLimits, SecretChatManager, SecretEvent, SecretTransport,
};
use kiaro_secret_chat::protocol::{ChatRole, ChatState, decrypt_frame, unwrap_frame};
use tokio::sync::{Mutex, Notify};

const KEY: [u8; 256] = [0x51; 256];

#[derive(Default)]
struct ControlledTransport {
    block_next: AtomicBool,
    fail_next: AtomicBool,
    entered: Notify,
    release: Notify,
    sent: Mutex<Vec<(i64, i64, Vec<u8>)>>,
}

impl SecretTransport for ControlledTransport {
    fn auth_identity(&self) -> &str {
        "offline-audit-authorization"
    }

    fn send_encrypted<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.sent.lock().await.push((chat_id, random_id, frame));
            if self.block_next.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(SecretChatError::ParameterRejected {
                    chat_id: Some(chat_id),
                    reason: "simulated transport loss after server acceptance".into(),
                });
            }
            Ok(())
        })
    }

    fn discard_encryption<'a>(
        &'a self,
        _chat_id: i64,
        _delete_history: bool,
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
}

async fn ready() -> (
    Arc<ControlledTransport>,
    Arc<SecretChatManager<ControlledTransport>>,
) {
    let transport = Arc::new(ControlledTransport::default());
    let manager = Arc::new(SecretChatManager::new(Arc::clone(&transport)));
    manager
        .register_established_chat(10, 20, 30, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    (transport, manager)
}

fn wire_sequence(frame: &[u8]) -> i64 {
    let body = decrypt_frame(&KEY, frame, 0).unwrap();
    unwrap_frame(&body, Some(10)).unwrap()["out_seq_no"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn concurrent_sends_wait_for_prior_transport_and_use_distinct_sequence_numbers() {
    let (transport, manager) = ready().await;
    transport.block_next.store(true, Ordering::SeqCst);
    let first_manager = Arc::clone(&manager);
    let first = tokio::spawn(async move { first_manager.send_text(10, 1, "first", 0).await });
    transport.entered.notified().await;

    let second = manager.send_text(10, 2, "second", 0);
    tokio::pin!(second);
    poll_fn(|cx| {
        assert!(
            second.as_mut().poll(cx).is_pending(),
            "same-chat send overtook an unfinished RPC"
        );
        Poll::Ready(())
    })
    .await;
    transport.release.notify_one();
    assert_eq!(wire_sequence(&first.await.unwrap().unwrap()), 1);
    assert_eq!(wire_sequence(&second.await.unwrap()), 3);
    assert_eq!(manager.out_seq_no(10).await.unwrap(), 2);
}

#[tokio::test]
async fn uncertain_transport_failure_reserves_sequence_and_retries_identical_ciphertext() {
    let (transport, manager) = ready().await;
    transport.fail_next.store(true, Ordering::SeqCst);
    assert!(matches!(
        manager.send_text(10, 7, "possibly accepted", 0).await,
        Err(SecretChatError::SendPending { .. })
    ));
    assert_eq!(
        manager.out_seq_no(10).await.unwrap(),
        1,
        "uncertain delivery must not reuse a sequence number"
    );
    let retry = manager
        .send_text(10, 7, "possibly accepted", 0)
        .await
        .unwrap();
    let sent = transport.sent.lock().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0].2, sent[1].2,
        "retry changed random padding or message identity"
    );
    assert_eq!(sent[0].2, retry);
    assert_eq!(manager.out_seq_no(10).await.unwrap(), 1);
}

#[tokio::test]
async fn peer_acknowledgement_settles_a_send_even_if_its_rpc_later_fails() {
    let (transport, alice) = ready().await;
    let bob = SecretChatManager::new(Arc::new(ControlledTransport::default()));
    bob.register_established_chat(10, 20, 40, ChatRole::Acceptor, &KEY)
        .await
        .unwrap();
    transport.block_next.store(true, Ordering::SeqCst);
    transport.fail_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&alice);
    let task = tokio::spawn(async move { sending.send_text(10, 99, "acknowledged", 0).await });
    transport.entered.notified().await;
    let original = transport.sent.lock().await[0].2.clone();
    bob.receive_frames(10, &original).await.unwrap();
    let ack = bob.send_text(10, 100, "received", 0).await.unwrap();
    alice.receive_frames(10, &ack).await.unwrap();
    transport.release.notify_one();
    assert_eq!(task.await.unwrap().unwrap(), original);
    assert_eq!(alice.retry_pending(10).await.unwrap(), 0);
    assert_eq!(alice.out_seq_no(10).await.unwrap(), 1);
}

#[tokio::test]
async fn cancellation_retains_the_reserved_frame_for_exact_retry() {
    let (transport, manager) = ready().await;
    transport.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let task = tokio::spawn(async move { sending.send_text(10, 8, "cancelled await", 0).await });
    transport.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(manager.out_seq_no(10).await.unwrap(), 1);
    manager
        .send_text(10, 8, "cancelled await", 0)
        .await
        .unwrap();
    let sent = transport.sent.lock().await;
    assert_eq!(sent[0].2, sent[1].2);
    assert_eq!(manager.out_seq_no(10).await.unwrap(), 1);
}

#[tokio::test]
async fn duplicate_chat_registration_cannot_reset_live_state_or_revive_tombstones() {
    let (_, manager) = ready().await;
    manager.send_text(10, 1, "first", 0).await.unwrap();
    assert!(
        manager
            .register_established_chat(10, 20, 30, ChatRole::Initiator, &KEY)
            .await
            .is_err()
    );
    assert_eq!(manager.out_seq_no(10).await.unwrap(), 1);
    manager.close_chat(10, false, "audit close").await.unwrap();
    assert!(
        manager
            .register_established_chat(10, 20, 30, ChatRole::Initiator, &KEY)
            .await
            .is_err()
    );
    assert_eq!(manager.chat_state(10).await.unwrap(), ChatState::Closed);
}

#[tokio::test]
async fn retry_identity_cannot_be_reused_for_different_plaintext() {
    let (transport, manager) = ready().await;
    transport.fail_next.store(true, Ordering::SeqCst);
    assert!(manager.send_text(10, 9, "original", 0).await.is_err());
    assert!(matches!(
        manager.send_text(10, 9, "replacement", 0).await,
        Err(SecretChatError::ParameterRejected { .. })
    ));
    assert_eq!(transport.sent.lock().await.len(), 1);
}

#[tokio::test]
async fn recovering_a_gap_delivers_every_buffered_message_in_order() {
    let (_, alice) = ready().await;
    let bob = SecretChatManager::new(Arc::new(ControlledTransport::default()));
    bob.register_established_chat(10, 20, 40, ChatRole::Acceptor, &KEY)
        .await
        .unwrap();
    let mut frames = Vec::new();
    for (id, text) in [(1, "first"), (2, "second"), (3, "third")] {
        frames.push(alice.send_text(10, id, text, 0).await.unwrap());
    }
    let _ = bob.receive_frame(10, &frames[2]).await;
    bob.receive_frame(10, &frames[0]).await.unwrap();
    bob.receive_frame(10, &frames[1]).await.unwrap();
    let texts: Vec<_> = bob
        .drain_events()
        .await
        .into_iter()
        .filter_map(|event| match event {
            SecretEvent::MessageReceived { message, .. } => message.text,
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["first", "second", "third"]);
}

#[tokio::test]
async fn an_unrelated_chat_can_send_while_another_transport_call_waits() {
    let (transport, manager) = ready().await;
    manager
        .register_established_chat(11, 21, 31, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    transport.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let first = tokio::spawn(async move { sending.send_text(10, 1, "blocked", 0).await });
    transport.entered.notified().await;
    manager.send_text(11, 2, "independent", 0).await.unwrap();
    transport.release.notify_one();
    first.await.unwrap().unwrap();
}

#[tokio::test]
async fn retention_refuses_new_sends_until_peer_acknowledgement_releases_capacity() {
    let transport = Arc::new(ControlledTransport::default());
    let alice = SecretChatManager::with_limits(
        Arc::clone(&transport),
        ManagerLimits {
            max_retained_messages: 1,
            max_retained_bytes: 4096,
        },
    )
    .unwrap();
    alice
        .register_established_chat(10, 20, 30, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    let bob = SecretChatManager::new(Arc::new(ControlledTransport::default()));
    bob.register_established_chat(10, 20, 40, ChatRole::Acceptor, &KEY)
        .await
        .unwrap();
    let first = alice.send_text(10, 1, "retained", 0).await.unwrap();
    assert!(alice.send_text(10, 2, "over budget", 0).await.is_err());
    assert_eq!(alice.out_seq_no(10).await.unwrap(), 1);
    bob.receive_frame(10, &first).await.unwrap();
    let ack = bob.send_text(10, 3, "ack", 0).await.unwrap();
    alice.receive_frame(10, &ack).await.unwrap();
    alice
        .send_text(10, 2, "allowed after ack", 0)
        .await
        .unwrap();
    assert_eq!(alice.out_seq_no(10).await.unwrap(), 2);
}

#[tokio::test]
async fn shutdown_prevents_a_waiting_sender_from_reviving_or_sending() {
    let (transport, manager) = ready().await;
    transport.block_next.store(true, Ordering::SeqCst);
    let sending = Arc::clone(&manager);
    let first = tokio::spawn(async move { sending.send_text(10, 1, "in flight", 0).await });
    transport.entered.notified().await;
    let second = manager.send_text(10, 2, "waiting", 0);
    tokio::pin!(second);
    poll_fn(|cx| {
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    manager.shutdown().await;
    transport.release.notify_one();
    assert!(matches!(
        first.await.unwrap(),
        Err(SecretChatError::ManagerStopping)
    ));
    assert!(matches!(
        second.await,
        Err(SecretChatError::ManagerStopping)
    ));
    assert_eq!(transport.sent.lock().await.len(), 1);
    assert_eq!(manager.chat_state(10).await.unwrap(), ChatState::Closed);
}
