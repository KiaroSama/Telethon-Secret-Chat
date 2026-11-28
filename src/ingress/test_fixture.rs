//! Offline release-boundary tests. Credentials are burned deterministic fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::{binding::AuthorizationBinding, custody::IngressLimits};
use grammers_mtsender::{SenderPool, UpdatesConfiguration, UpdatesReceiver};
use grammers_session::{
    SessionData,
    storages::MemorySession,
    updates::{MessageBoxes, UpdatesLike},
};
use grammers_tl_types as tl;
use std::{sync::Arc, time::Duration};

pub(super) async fn fixture(
    records: usize,
) -> (tempfile::TempDir, Arc<DurableIngress>, SenderPool) {
    let (dir, ingress, pool, baseline) = unprepared(records);
    ingress.prepare(&pool.handle, Some(baseline)).await.unwrap();
    // Receipt-only tests use an internal completed-proof fixture. Public proof
    // refusal and the original Client harness are exercised independently.
    ingress.core.lock().unwrap().verified = true;
    (dir, ingress, pool)
}
pub(super) fn unprepared(
    records: usize,
) -> (tempfile::TempDir, Arc<DurableIngress>, SenderPool, Vec<u8>) {
    let base = std::env::temp_dir().canonicalize().unwrap();
    let dir = tempfile::Builder::new().tempdir_in(base).unwrap();
    let pool = pool();
    let client = grammers_client::Client::new(pool.handle.clone());
    let (dc_id, tag) = client.authorization_key_tag().unwrap().unwrap();
    let binding = AuthorizationBinding::v2(123, dc_id, tag, "burned-fixture").unwrap();
    let store = EncryptedIngressStorage::open(
        dir.path().join("state/custody.bin"),
        [9; 32],
        binding,
        1024 * 1024,
    )
    .unwrap();
    let ingress = DurableIngress::new(
        store,
        IngressLimits::new(records, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let mut machine = MessageBoxes::new();
    machine.set_state(tl::types::updates::State {
        pts: 10,
        qts: 40,
        date: 123,
        seq: 1,
        unread_count: 0,
    });
    (dir, ingress, pool, machine.durable_checkpoint().unwrap())
}
pub(super) fn pool() -> SenderPool {
    let mut session = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    session.dc_options.get_mut(&2).unwrap().auth_key = Some(std::array::from_fn(|i| i as u8));
    SenderPool::new(Arc::new(MemorySession::from(session)), 123)
}
pub(super) fn message(identity: i64, bytes: Vec<u8>) -> tl::enums::EncryptedMessage {
    tl::types::EncryptedMessageService {
        random_id: identity,
        chat_id: 7,
        date: 124,
        bytes,
    }
    .into()
}
pub(super) fn live(qts: i32, seq: i32, bytes: Vec<u8>) -> UpdatesLike {
    UpdatesLike::Updates(
        tl::types::Updates {
            updates: vec![
                tl::types::UpdateNewEncryptedMessage {
                    message: message(qts as i64, bytes),
                    qts,
                }
                .into(),
            ],
            users: vec![],
            chats: vec![],
            date: 124,
            seq,
        }
        .into(),
    )
}
pub(super) fn difference(qts: i32) -> tl::enums::updates::Difference {
    tl::types::updates::Difference {
        topics: None,
        new_messages: vec![],
        new_encrypted_messages: vec![
            message(51, b"complete qts-less first".to_vec()),
            message(52, b"complete qts-less second".to_vec()),
        ],
        other_updates: vec![],
        chats: vec![],
        users: vec![],
        state: tl::types::updates::State {
            pts: 10,
            qts,
            date: 124,
            seq: 2,
            unread_count: 0,
        }
        .into(),
    }
    .into()
}
pub(super) fn request(qts: i32) -> tl::functions::updates::GetDifference {
    tl::functions::updates::GetDifference {
        pts: 10,
        pts_limit: None,
        pts_total_limit: None,
        date: 123,
        qts,
        qts_limit: None,
    }
}
pub(super) async fn receiver(
    ingress: &Arc<DurableIngress>,
    pool: &SenderPool,
) -> (tokio::sync::mpsc::Sender<UpdatesLike>, UpdatesReceiver) {
    assert!(ingress.core.lock().unwrap().attached);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let receiver = UpdatesReceiver::create(
        pool.handle.thin.clone(),
        pool.handle.session.clone(),
        rx,
        UpdatesConfiguration::default(),
    )
    .await
    .unwrap();
    (tx, receiver)
}
pub(super) async fn capture(
    receiver: &mut UpdatesReceiver,
    tx: &tokio::sync::mpsc::Sender<UpdatesLike>,
    update: UpdatesLike,
) {
    tx.send(update).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), receiver.next())
            .await
            .unwrap()
            .is_ok()
    );
}
