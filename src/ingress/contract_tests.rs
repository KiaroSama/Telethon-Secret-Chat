//! Ports of receipt reconstruction and public attachment refusal scenarios.
#![allow(clippy::unwrap_used)]
use super::{DurableIngress, EncryptedIngressStorage, test_fixture::*};
use crate::{
    custody::IngressLimits,
    storage::{CommitFault, Storage},
};
use grammers_mtsender::DurableIngressHook;
use grammers_session::updates::{MessageBoxes, UpdatesLike};
use grammers_tl_types::{self as tl, Serializable};
use std::{sync::Arc, time::Duration};

fn qts(ingress: &DurableIngress) -> i32 {
    MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
        .unwrap()
        .session_state()
        .qts
}
fn restore(
    path: &std::path::Path,
    binding: crate::binding::AuthorizationBinding,
) -> Arc<DurableIngress> {
    DurableIngress::new(
        EncryptedIngressStorage::open(path, [9; 32], binding, 1024 * 1024).unwrap(),
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn unwrapped_ack_cannot_exceed_committed_frontier() {
    let (_dir, ingress, _pool) = fixture(32).await;
    assert!(ingress.guard_request(2, &request(41).to_bytes()).is_err());
    ingress.fence().unwrap();
    assert_eq!(qts(&ingress), 40);
}
#[tokio::test]
async fn business_and_secret_updates_share_nonzero_qts_frontier() {
    let (_dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let update = UpdatesLike::Updates(
        tl::types::Updates {
            updates: vec![
                tl::types::UpdateBotNewBusinessMessage {
                    connection_id: "public fixture".into(),
                    message: tl::types::MessageEmpty {
                        id: 1,
                        peer_id: None,
                    }
                    .into(),
                    reply_to_message: None,
                    qts: 41,
                }
                .into(),
                tl::types::UpdateNewEncryptedMessage {
                    message: message(42, b"secret fixture".to_vec()),
                    qts: 42,
                }
                .into(),
            ],
            users: vec![],
            chats: vec![],
            date: 124,
            seq: 2,
        }
        .into(),
    );
    capture(&mut receiver, &tx, update).await;
    assert_eq!(qts(&ingress), 42);
    let batches = ingress.pending_batches().unwrap();
    let updates = batches[0].updates().unwrap();
    assert_eq!(updates.0.len(), 2);
    assert!(
        matches!(&updates.0[0].0,tl::enums::Update::BotNewBusinessMessage(item) if item.qts==41)
    );
    assert!(matches!(&updates.0[1].0,tl::enums::Update::NewEncryptedMessage(item) if item.qts==42));
    assert!(ingress.guard_request(2, &request(42).to_bytes()).is_ok());
}
#[tokio::test]
async fn difference_slices_advance_only_their_committed_state() {
    let (_dir, ingress, _pool) = fixture(32).await;
    for (current, next, final_slice) in [(40, 41, false), (41, 42, true)] {
        let state = tl::types::updates::State {
            pts: 10,
            qts: next,
            date: 124,
            seq: 1,
            unread_count: 0,
        };
        let response: tl::enums::updates::Difference = if final_slice {
            tl::types::updates::Difference {
                new_messages: vec![],
                new_encrypted_messages: vec![],
                other_updates: vec![],
                chats: vec![],
                users: vec![],
                state: state.into(),
            }
            .into()
        } else {
            tl::types::updates::DifferenceSlice {
                new_messages: vec![],
                new_encrypted_messages: vec![],
                other_updates: vec![],
                chats: vec![],
                users: vec![],
                intermediate_state: state.into(),
            }
            .into()
        };
        let body = request(current).to_bytes();
        let permit = ingress.guard_request(2, &body).unwrap();
        ingress
            .guard_result(permit, 2, &body, &response.to_bytes())
            .unwrap();
        assert_eq!(qts(&ingress), next);
    }
    let batches = ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(batches[0].after.as_bytes())
            .unwrap()
            .session_state()
            .qts,
        41
    );
}
#[tokio::test]
async fn impossible_live_gap_is_held_without_inventing_qts() {
    super::strong_tests::review::materialized_output_gets_a_fresh_receipt();
    let (_dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    tx.send(live(42, 2, b"public gap fixture".to_vec()))
        .await
        .unwrap();
    // The pinned SDK emits an empty metadata tuple for a buffered possible gap.
    // Consume that documented result, rejecting any invented update or peer.
    let buffered = tokio::time::timeout(Duration::from_secs(1), receiver.next())
        .await
        .unwrap()
        .unwrap();
    assert!(buffered.0.is_empty());
    assert!(buffered.1.is_empty());
    assert!(buffered.2.is_empty());
    // The next receive must actually wait for recovery rather than immediately
    // yielding another tuple or refusing the otherwise healthy receiver.
    assert!(
        tokio::time::timeout(Duration::from_millis(25), receiver.next())
            .await
            .is_err()
    );
    assert_eq!(qts(&ingress), 40);
    assert!(ingress.guard_request(2, &request(42).to_bytes()).is_err());
    assert_eq!(ingress.pending_batches().unwrap().len(), 1);
    tx.send(live(41, 2, b"missing predecessor fixture".to_vec()))
        .await
        .unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(1), receiver.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered
            .0
            .iter()
            .map(|(update, _)| match update {
                tl::enums::Update::NewEncryptedMessage(update) => update.message.random_id(),
                _ => panic!("expected the actual predecessor and held encrypted update"),
            })
            .collect::<Vec<_>>(),
        [41, 42]
    );
    assert_eq!(qts(&ingress), 42);
    let permit = ingress
        .guard_request(
            2,
            &tl::functions::messages::ReceivedQueue { max_qts: 42 }.to_bytes(),
        )
        .unwrap();
    ingress.cancel_request(permit);
}
#[tokio::test]
async fn acknowledged_receipt_reconstructs_before_consumer_application() {
    let (dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    capture(
        &mut receiver,
        &tx,
        live(41, 2, b"public retained fixture".to_vec()),
    )
    .await;
    let binding = ingress.binding().unwrap();
    let old = ingress.pending_batches().unwrap()[0].clone();
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes()
            )
            .is_ok()
    );
    let owner = Arc::downgrade(&ingress);
    drop(receiver);
    drop(pool);
    drop(ingress);
    assert!(
        owner.upgrade().is_none(),
        "completed SDK callbacks must release the custody owner before reopen"
    );
    let restored = restore(&dir.path().join("state/custody.bin"), binding);
    assert_eq!(qts(&restored), 41);
    let rows = restored.pending_batches().unwrap();
    assert_eq!(rows[0].identity, old.identity);
    assert_eq!(rows[0].envelope.as_bytes(), old.envelope.as_bytes());
    assert_eq!(rows[0].cursor, 0);
}
async fn uncertain_commit(fault: CommitFault, expected: i32) {
    let (dir, ingress, pool) = fixture(32).await;
    let binding = ingress.binding().unwrap();
    let (_tx, mut receiver) = receiver(&ingress, &pool).await;
    let mut storage = ingress.protocol_storage().unwrap();
    ingress.core.lock().unwrap().backend.fail_next_commit(fault);
    assert!(
        ingress
            .stage_live(2, &live(41, 2, b"uncertain public fixture".to_vec()))
            .is_err()
    );
    assert!(receiver.next().await.is_err());
    assert!(storage.load().is_err());
    assert!(storage.resume_storage().is_err());
    assert!(storage.committed_view().is_none());
    assert_eq!(qts(&ingress), 40);
    assert!(ingress.guard_request(2, &request(41).to_bytes()).is_err());
    let owner = Arc::downgrade(&ingress);
    drop(storage);
    drop(receiver);
    drop(pool);
    drop(ingress);
    assert!(
        owner.upgrade().is_none(),
        "the original SDK handles must release the custody owner before reopen"
    );
    let restored = restore(&dir.path().join("state/custody.bin"), binding);
    // New SDK capture persists the whole raw body before applying MessageBoxes.
    // An uncertain raw receipt may reopen at qts40 with that input safely held.
    assert_eq!(qts(&restored), 40);
    let raw = restored.core.lock().unwrap().ledger.pending_live().unwrap();
    assert_eq!(raw.len(), usize::from(expected == 41));
    if !raw.is_empty() {
        assert!(matches!(&raw[0], UpdatesLike::Updates(_)));
        let next_pool = super::test_fixture::pool();
        restored.prepare(&next_pool.handle, None).await.unwrap();
        let (_tx, mut receiver) = super::test_fixture::receiver(&restored, &next_pool).await;
        let output = tokio::time::timeout(Duration::from_secs(1), receiver.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.0.len(), 1);
        assert_eq!(qts(&restored), 41);
        assert_eq!(restored.pending_batches().unwrap().len(), 1);
        assert!(
            restored.guard_request(2, &request(41).to_bytes()).is_err(),
            "reconstruction requires a fresh proof before acknowledging"
        );
    }
}
#[tokio::test]
async fn failure_before_atomic_replace_restores_previous_exact_state() {
    uncertain_commit(CommitFault::BeforeReplace, 40).await;
}
#[tokio::test]
async fn failure_after_atomic_replace_restores_whole_raw_receipt() {
    uncertain_commit(CommitFault::AfterReplace, 41).await;
}
#[tokio::test]
async fn consumed_difference_result_handoff_cannot_recapture_receipt() {
    let (_dir, ingress, pool) = fixture(32).await;
    let (_tx, mut receiver) = receiver(&ingress, &pool).await;
    let body = request(40).to_bytes();
    let response: tl::enums::updates::Difference =
        tl::types::updates::DifferenceEmpty { date: 123, seq: 1 }.into();
    let permit = ingress.guard_request(2, &body).unwrap();
    ingress
        .guard_result(permit, 2, &body, &response.to_bytes())
        .unwrap();
    assert_eq!(ingress.pending_batches().unwrap().len(), 1);
    ingress.prune_completed().unwrap();
    assert!(ingress.pending_batches().unwrap().is_empty());
    let _ = tokio::time::timeout(Duration::from_millis(25), receiver.next()).await;
    assert_eq!(qts(&ingress), 40);
    assert!(ingress.pending_batches().unwrap().is_empty());
}
#[tokio::test]
async fn session_projection_uses_custody_frontier_after_stop() {
    let (dir, ingress, pool) = fixture(32).await;
    let mut observed = MessageBoxes::new();
    observed.set_state(tl::types::updates::State {
        pts: 10,
        qts: 999,
        date: 124,
        seq: 2,
        unread_count: 0,
    });
    pool.handle
        .session
        .set_update_state(grammers_session::types::UpdateState::All(
            observed.session_state(),
        ))
        .await
        .unwrap();
    assert_eq!(pool.handle.session.updates_state().await.unwrap().qts, 999);
    let (_tx, sdk_receiver) = receiver(&ingress, &pool).await;
    let binding = ingress.binding().unwrap();
    ingress.fence().unwrap();
    sdk_receiver.sync_update_state().await.unwrap();
    assert_eq!(pool.handle.session.updates_state().await.unwrap().qts, 40);
    assert_eq!(qts(&ingress), 40);
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.restore().unwrap())
            .unwrap()
            .session_state()
            .qts,
        40
    );
    drop(sdk_receiver);
    drop(pool);
    drop(ingress);
    let restored = restore(&dir.path().join("state/custody.bin"), binding);
    assert_eq!(qts(&restored), 40);
    assert_eq!(observed.session_state().qts, 999);
}
