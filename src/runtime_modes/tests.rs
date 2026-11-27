use super::*;
use crate::protocol::{ChatRole, encrypt_frame, wrap_frame};
use crate::storage_backend::MemoryStorage;

struct Transport;

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        "durable-event-audit"
    }

    fn send_encrypted<'a>(&'a self, _: i64, _: i64, _: Vec<u8>) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn durable_publication_does_not_retain_inaccessible_ram_events() {
    let key = [0x63; 256];
    let manager = PersistentSecretChatManager::open(
        Transport,
        MemoryStorage::new("durable-event-audit").unwrap(),
        ManagerLimits::default(),
    )
    .await
    .unwrap();
    manager
        .register_and_persist(
            PersistedChatRecord::established(7, 8, 9, ChatRole::Acceptor, key).unwrap(),
        )
        .await
        .unwrap();
    let outbound = manager.send_text_with_id(7, 11, "outbound").await.unwrap();
    let mut peer = SecretChat::new(7, 8, 9, "peer", ChatRole::Initiator);
    peer.adopt_key(&key).unwrap();
    peer.receive_encrypted_frames(&outbound).unwrap();
    let service = peer
        .prepare_service_action(&json!({"id": TTL_ACTION, "fields": {"ttl_seconds": 9}}))
        .unwrap();
    assert_eq!(
        manager
            .handle_incoming(7, &service.frame)
            .await
            .unwrap()
            .len(),
        1
    );
    let body = wrap_frame(
        &json!({"random_id": 12, "ttl": 9, "message": "inbound"}),
        73,
        2,
        3,
    )
    .unwrap();
    let frame = encrypt_frame(&key, &body, 0).unwrap();
    assert_eq!(manager.handle_incoming(7, &frame).await.unwrap().len(), 1);
    manager.close_and_tombstone(7, false).await.unwrap();
    assert!(
        manager.inner.drain_events().await.is_empty(),
        "durable receipts must not also accumulate in an inaccessible RAM queue"
    );
}
