use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, CoreSecretChatManager, SecretEvent, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, SecretChat};
use serde_json::json;
use tokio::sync::{Mutex, Notify};

const KEY: [u8; 256] = [0x39; 256];

#[derive(Default)]
struct ScheduledTransport {
    blocked: AtomicBool,
    fail_next: AtomicBool,
    entered: Notify,
    release: Notify,
    frames: Mutex<Vec<Vec<u8>>>,
}

impl SecretTransport for ScheduledTransport {
    fn auth_identity(&self) -> &str {
        "offline-manager-followup"
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.frames.lock().await.push(frame);
            if self.blocked.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(SecretChatError::ManagerStopping);
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

    fn discard_encryption<'a>(
        &'a self,
        _: i64,
        _: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

async fn replay_while_sending(fail_original: bool, duplicate_during_replay: bool) {
    let transport = Arc::new(ScheduledTransport::default());
    let alice = Arc::new(CoreSecretChatManager::new(transport.clone()));
    alice
        .register_established_chat(9, 10, 11, ChatRole::Initiator, &KEY)
        .await
        .unwrap();
    let mut bob = SecretChat::new(9, 10, 12, "peer", ChatRole::Acceptor);
    bob.adopt_key(&KEY).unwrap();

    transport.blocked.store(true, Ordering::SeqCst);
    transport.fail_next.store(fail_original, Ordering::SeqCst);
    let sending = alice.clone();
    let task = tokio::spawn(async move { sending.send_text(9, 17, "retained message", 0).await });
    transport.entered.notified().await;
    let original = transport.frames.lock().await[0].clone();
    bob.receive_encrypted_frames(&original).unwrap();
    let resend = bob
        .prepare_service_action(
            &json!({"id":0x511110b0_u32,"fields":{"start_seq_no":1,"end_seq_no":1}}),
        )
        .unwrap();

    // This authenticated request also acknowledges the original sequence position.
    // The concurrent send must preserve the separate obligation to replay its bytes.
    alice.receive_frames(9, &resend.frame).await.unwrap();
    if duplicate_during_replay {
        transport.blocked.store(true, Ordering::SeqCst);
    }
    transport.release.notify_one();
    if fail_original {
        assert!(matches!(
            task.await.unwrap(),
            Err(SecretChatError::SendPending { random_id: 17, .. })
        ));
        assert_eq!(alice.retry_pending(9).await.unwrap(), 1);
    } else {
        if duplicate_during_replay {
            transport.entered.notified().await;
            assert!(
                alice
                    .receive_frames(9, &resend.frame)
                    .await
                    .unwrap()
                    .is_empty()
            );
            transport.release.notify_one();
        }
        assert_eq!(task.await.unwrap().unwrap(), original);
    }

    assert_eq!(
        transport.frames.lock().await.as_slice(),
        [original.clone(), original]
    );
    assert_eq!(alice.out_seq_no(9).await.unwrap(), 1);
    assert_eq!(alice.retry_pending(9).await.unwrap(), 0);
    let acknowledged = alice
        .drain_events()
        .await
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                SecretEvent::MessageAcknowledged { random_id: 17, .. }
            )
        })
        .count();
    assert_eq!(acknowledged, 1);
}

#[tokio::test]
async fn authenticated_resend_survives_completion_of_an_older_in_flight_rpc() {
    tokio::time::timeout(Duration::from_secs(10), replay_while_sending(false, false))
        .await
        .expect("in-flight replay schedule deadlocked");
}

#[tokio::test]
async fn peer_acknowledgement_does_not_clear_replay_when_original_rpc_fails() {
    tokio::time::timeout(Duration::from_secs(10), replay_while_sending(true, false))
        .await
        .expect("failed replay schedule deadlocked");
}

#[tokio::test]
async fn duplicate_resend_during_replay_does_not_queue_another_transmission() {
    tokio::time::timeout(Duration::from_secs(10), replay_while_sending(false, true))
        .await
        .expect("duplicate replay schedule deadlocked");
}
