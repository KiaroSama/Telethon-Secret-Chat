#![allow(clippy::unwrap_used)]
use crate::custody::IngressLimits;
use crate::ingress::{
    coordinator::DurableIngress,
    ledger::{Ledger, now},
    test_fixture::*,
};
use grammers_mtsender::{DurableIngressHook, IngressReceiptKind};
use grammers_session::updates::{MessageBoxes, UpdatesLike, durable};
use grammers_tl_types::{self as tl, Serializable};

fn cloud() -> UpdatesLike {
    UpdatesLike::Updates(
        tl::types::Updates {
            updates: vec![
                tl::types::UpdateDeleteMessages {
                    messages: vec![88],
                    pts: 11,
                    pts_count: 1,
                }
                .into(),
            ],
            users: vec![],
            chats: vec![],
            date: 124,
            seq: 2,
        }
        .into(),
    )
}
pub(crate) async fn expired_receipts_reopen_and_drain() {
    for raw in [false, true] {
        let (dir, ingress, pool) = fixture(32).await;
        let binding = ingress.binding().unwrap();
        let (tx, mut receiver) = receiver(&ingress, &pool).await;
        if raw {
            ingress.stage_live(2, &cloud()).unwrap();
        } else {
            capture(&mut receiver, &tx, cloud()).await;
        }
        {
            let mut core = ingress.core.lock().unwrap();
            core.limits.age_seconds = 0.01;
            let mut serialized = serde_json::to_value(&core.ledger).unwrap();
            for group in ["inputs", "batches"] {
                for row in serialized[group].as_array_mut().unwrap() {
                    row["created_ms"] = (now().unwrap() - 1000).into();
                }
            }
            let ledger = serde_json::from_value(serialized).unwrap();
            let state = core.state.clone();
            core.persist(state, ledger).unwrap();
        }
        drop(receiver);
        drop(tx);
        drop(pool);
        drop(ingress);
        let backend = crate::ingress::EncryptedIngressStorage::open(
            dir.path().join("state/custody.bin"),
            [9; 32],
            binding,
            1024 * 1024,
        )
        .unwrap();
        let restored =
            DurableIngress::new(backend, IngressLimits::new(32, 1024 * 1024, 0.01).unwrap())
                .unwrap();
        if raw {
            let pool = super::super::test_fixture::pool();
            restored.prepare(&pool.handle, None).await.unwrap();
            restored.core.lock().unwrap().verified = true;
            let (_tx, mut receiver) = super::super::test_fixture::receiver(&restored, &pool).await;
            let output = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.next())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output.0.len(), 1);
        }
        let held = restored.pending_batches().unwrap();
        assert_eq!(held.len(), 1);
        restored.acknowledge_nonsecret(held[0].id, 0).unwrap();
        restored.prune_completed().unwrap();
        assert!(restored.pending_batches().unwrap().is_empty());
    }
}

pub(crate) fn materialized_output_gets_a_fresh_receipt() {
    let mut machine = MessageBoxes::new();
    machine.set_state(tl::types::updates::State {
        pts: 10,
        qts: 40,
        date: 123,
        seq: 1,
        unread_count: 0,
    });
    let checkpoint = machine.durable_checkpoint().unwrap();
    let limits = IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap();
    let envelope = durable::encode_live(&cloud()).unwrap();
    let mut ledger = Ledger::default();
    ledger.bootstrap(checkpoint.clone()).unwrap();
    let empty = (vec![], vec![], vec![]);
    let first = ledger
        .capture(
            (&checkpoint, &checkpoint),
            IngressReceiptKind::Live,
            &envelope,
            &[],
            &empty,
            &limits,
        )
        .unwrap()
        .unwrap();
    let output = machine.process_updates(cloud()).unwrap();
    let after = machine.durable_checkpoint().unwrap();
    let second = ledger
        .capture(
            (&checkpoint, &after),
            IngressReceiptKind::Live,
            &envelope,
            &[],
            &output,
            &limits,
        )
        .unwrap()
        .unwrap();
    assert_ne!(
        first, second,
        "an already observed empty receipt ID cannot hide new output"
    );
    assert_eq!(ledger.batches.len(), 1);
    assert_eq!(ledger.batches[0].updates().unwrap().0.len(), 1);
    assert_eq!(ledger.checkpoint().unwrap(), after);
}

pub(crate) async fn stale_difference_retries_at_current_pts() {
    let (_dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let request = request(40).to_bytes();
    let permit = ingress.guard_request(2, &request).unwrap();
    capture(&mut receiver, &tx, cloud()).await;
    let before = ingress.checkpoint().unwrap();
    let mut response = difference(42);
    if let tl::enums::updates::Difference::Difference(diff) = &mut response {
        let tl::enums::updates::State::State(state) = &mut diff.state;
        state.pts = 11;
    }
    assert!(
        ingress
            .guard_result(permit, 2, &request, &response.to_bytes())
            .is_err()
    );
    assert_eq!(ingress.checkpoint().unwrap(), before);
    assert!(
        !ingress.core.lock().unwrap().failed,
        "a stale caller result must permit a fresh retry"
    );
    let mut fresh = super::super::test_fixture::request(40);
    fresh.pts = 11;
    let fresh = fresh.to_bytes();
    let permit = ingress.guard_request(2, &fresh).unwrap();
    ingress
        .guard_result(permit, 2, &fresh, &response.to_bytes())
        .unwrap();
    let received = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.0.len(), 2);
}
