#![allow(clippy::unwrap_used)]
use super::*;
#[path = "../../../tests/support_manager/runtime_review.rs"]
mod review885;
use crate::test_support::support;
use crate::{
    custody::{TransientLimits, TransientStorage},
    storage::Storage,
};
#[tokio::test]
async fn nested_reservations_reuse_task_and_release_each_payload() {
    crate::media::SourceReader::verify_refill_ownership().await;
    review885::ea3_regressions().await;
    review885::acknowledged_ids_are_budgeted();
    review885::callback_admission_is_all_or_nothing();
    review885::cleanup_panic_releases_registry().await;
    review885::fencing_waits_for_runtime_publication().await;
    bounded_headroom_tracks_nested_and_concurrent_owners().await;
    interrupted_creations_keep_uncertain_identity_quarantined().await;
    let (transport, _) = support::Fake::pair();
    let store = support::TestStorage::default();
    let limits = TransientLimits {
        bytes: 65536,
        frame_bytes: 4096,
        tasks: 1,
        retention_seconds: 0.05,
        ..Default::default()
    };
    let manager = SecretChatManager::new(
        transport,
        TransientStorage::new(store.clone(), "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    validate_runtime_budget_and_split_ownership(manager.inner.work.policy.unwrap());
    let base = serde_json::to_vec(&store.load().unwrap().unwrap())
        .unwrap()
        .len()
        * 4;
    blocking_workers_retain_operation_leases(&manager, base).await;
    manager
        .with_payload_operation(None, WorkKind::Ordinary, 1024, async {
            manager
                .with_payload_operation(None, WorkKind::Ordinary, 2048, async {
                    let held = manager.reserve_payload(None, 65536 - base - 1024 - 2048)?;
                    assert!(manager.reserve_payload(None, 1).is_err());
                    drop(held);
                    Ok(())
                })
                .await?;
            let held = manager.reserve_payload(None, 65536 - base - 1024)?;
            assert!(manager.reserve_payload(None, 1).is_err());
            drop(held);
            assert!(
                manager
                    .with_payload_operation(None, WorkKind::Ordinary, 65536, async {
                        Ok::<(), Error>(())
                    })
                    .await
                    .is_err()
            );
            Ok(())
        })
        .await
        .unwrap();
    let held = manager.reserve_payload(None, 65536 - base).unwrap();
    let generation = manager.inner.creation_generation.load(Ordering::Acquire);
    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.inner.creation_generation.load(Ordering::Acquire) == generation {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        manager.reserve_payload(None, 1).is_err(),
        "an expired held payload stays charged until Drop"
    );
    drop(held);
    let released = manager.reserve_payload(None, 65536 - base).unwrap();
    drop(released);
    manager.stop().await.unwrap();
}

fn validate_runtime_budget_and_split_ownership(policy: RuntimePolicy) {
    for invalid in [
        RuntimePolicy {
            operation_seconds: f64::NAN,
            ..policy
        },
        RuntimePolicy {
            operation_seconds: 0.0,
            ..policy
        },
        RuntimePolicy {
            operation_seconds: 61.0,
            ..policy
        },
        RuntimePolicy {
            cleanup_seconds: f64::INFINITY,
            ..policy
        },
        RuntimePolicy {
            cleanup_seconds: 0.0,
            ..policy
        },
        RuntimePolicy {
            cleanup_seconds: 31.0,
            ..policy
        },
        RuntimePolicy { tasks: 0, ..policy },
        RuntimePolicy {
            tasks: 65,
            ..policy
        },
    ] {
        assert!(WorkBudget::new(Some(invalid)).is_err());
    }
    let budget = WorkBudget::new(Some(RuntimePolicy { tasks: 2, ..policy })).unwrap();
    let mut all = budget.acquire(2).unwrap();
    assert!(budget.acquire(1).is_err());
    let first = all.take_one().unwrap();
    let second = all.take_one().unwrap();
    assert!(all.take_one().is_none());
    drop(all);
    assert!(
        budget.acquire(1).is_err(),
        "splitting transfers ownership rather than releasing it"
    );
    drop(first);
    let replacement = budget.acquire(1).unwrap();
    assert!(budget.acquire(usize::MAX).is_err());
    drop((second, replacement));
    assert_eq!(budget.active.load(Ordering::Acquire), 0);
    drop(budget.acquire(2).unwrap());
}
async fn bounded_headroom_tracks_nested_and_concurrent_owners() {
    use crate::custody::{CustodyLimits, CustodyStorage};
    let (transport, _) = support::Fake::pair();
    let world = Arc::clone(&transport.world);
    let store = CustodyStorage::new(
        support::TestStorage::default(),
        "11".repeat(32),
        CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let manager =
        SecretChatManager::new(transport, store, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    manager
        .protocol(|p| p.insert_chat(crate::test_support::seeded::chat()))
        .unwrap();
    manager
        .with_operation(Some(7), WorkKind::Ordinary, async {
            let outer = manager.reserve_headroom(7, 4, 4096)?;
            let inner = manager.reserve_headroom(7, 2, 2048)?;
            drop(inner);
            assert_eq!(
                manager
                    .inner
                    .headroom
                    .lock()
                    .unwrap()
                    .values()
                    .map(Headroom::maximum)
                    .collect::<Vec<_>>(),
                vec![(4, 4096)]
            );
            let other = manager.clone();
            tokio::spawn(async move {
                other
                    .with_operation(Some(7), WorkKind::Ordinary, async {
                        let concurrent = other.reserve_headroom(7, 2, 2048)?;
                        assert_eq!(other.inner.headroom.lock().unwrap().len(), 2);
                        drop(concurrent);
                        Ok(())
                    })
                    .await
            })
            .await
            .unwrap()?;
            assert_eq!(manager.inner.headroom.lock().unwrap().len(), 1);
            drop(outer);
            assert!(manager.inner.headroom.lock().unwrap().is_empty());
            let first = manager.reserve_headroom(7, 4, 4096)?;
            let second = manager.reserve_headroom(7, 2, 2048)?;
            drop(first);
            assert_eq!(
                manager
                    .inner
                    .headroom
                    .lock()
                    .unwrap()
                    .values()
                    .map(Headroom::maximum)
                    .collect::<Vec<_>>(),
                vec![(2, 2048)],
                "retained workers may release nested headroom out of stack order"
            );
            drop(second);
            assert!(
                manager.inner.headroom.lock().unwrap().is_empty(),
                "an older lease cannot resurrect released headroom"
            );
            Ok(())
        })
        .await
        .unwrap();
    // A public operation nested under unbound owned work remains cancellable
    // by an explicit operator pause without losing its durable RPC identity.
    world
        .lock()
        .unwrap()
        .requests
        .insert(7, (1000, 2000, vec![]));
    let gate = Arc::new(support::Gate::default());
    world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let paused = manager.clone();
    let pausing = tokio::spawn(async move {
        gate.entered.notified().await;
        paused.pause_chat(7).unwrap();
    });
    manager
        .with_operation(None, WorkKind::Ordinary, async {
            assert!(matches!(
                manager
                    .send_message(7, "nested owned send", None, None)
                    .await,
                Err(Error::Paused)
            ));
            Ok(())
        })
        .await
        .unwrap();
    pausing.await.unwrap();
    let before = world.lock().unwrap().sent.len();
    assert_eq!(before, 1);
    assert_eq!(world.lock().unwrap().active_sends, 0);
    assert!(manager.status(7).unwrap().paused);
    assert_eq!(
        manager
            .protocol(|p| Ok(p
                .chat_record(7)?
                .outbox
                .values()
                .filter(|row| row.pending)
                .count()))
            .unwrap(),
        1
    );
    assert!(
        manager
            .send_message(7, "still paused", None, None)
            .await
            .is_err()
    );
    assert_eq!(world.lock().unwrap().sent.len(), before);
    assert!(world.lock().unwrap().discards.is_empty());
    manager.stop().await.unwrap();
}
async fn interrupted_creations_keep_uncertain_identity_quarantined() {
    for deadline in [false, true] {
        let (transport, _) = support::Fake::pair();
        let world = Arc::clone(&transport.world);
        let gate = Arc::new(support::Gate::default());
        let wait = Arc::clone(&gate);
        world.lock().unwrap().request_hook = Some(Box::new(move |_| {
            Box::pin(async move {
                wait.entered.notify_one();
                wait.release.notified().await;
            })
        }));
        let store = support::TestStorage::default();
        let limits = TransientLimits {
            operation_seconds: if deadline { 0.04 } else { 2.0 },
            ..Default::default()
        };
        let manager = SecretChatManager::new(
            transport,
            TransientStorage::new(store.clone(), "11".repeat(32), limits).unwrap(),
            "11".repeat(32),
            Options::default(),
        )
        .unwrap();
        manager.start().await.unwrap();
        let generation = manager.inner.creation_generation.load(Ordering::Acquire);
        let creating = manager.clone();
        let task = tokio::spawn(async move { creating.create(support::peer(2000)).await });
        tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
            .await
            .unwrap();
        if deadline {
            assert!(task.await.unwrap().is_err());
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert!(manager.inner.creation_generation.load(Ordering::Acquire) > generation);
        assert!(
            manager
                .protocol(|p| Ok(p.private_record_names("pending-create/").is_empty()))
                .unwrap()
        );
        assert_eq!(
            manager
                .protocol(|p| Ok(p.private_record_names("suspended-create/").len()))
                .unwrap(),
            1
        );
        assert!(manager.list().unwrap().is_empty());
        gate.release.notify_one();
        tokio::task::yield_now().await;
        assert_eq!(world.lock().unwrap().request_ids.len(), 1);
        manager.stop().await.unwrap();
        assert_eq!(
            store
                .load()
                .unwrap()
                .unwrap()
                .private_records
                .keys()
                .filter(|key| key.starts_with("suspended-create/"))
                .count(),
            1
        );
    }
}

async fn blocking_workers_retain_operation_leases(
    manager: &SecretChatManager<TransientStorage<support::TestStorage>, support::Fake>,
    used: usize,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker_manager = manager.clone();
    let operation = tokio::spawn(async move {
        worker_manager
            .with_payload_operation(None, WorkKind::Ordinary, 1024, async {
                let _extra = worker_manager.reserve_payload(None, 2048)?;
                crate::media::blocking::spawn(move |_| {
                    let _ = entered_tx.send(());
                    release_rx.recv().unwrap();
                })?
                .await
                .map_err(|_| Error::Storage)?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered_rx)
        .await
        .unwrap()
        .unwrap();
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    assert!(
        manager.inner.work.acquire(1).is_err(),
        "canceling an await cannot release the task permit still owned by its worker"
    );
    let state = manager.protocol(|p| Ok(p.chat_records())).unwrap();
    assert!(state.is_empty());
    let available = 65536 - used - 1024 - 2048;
    let spare = manager.reserve_payload(None, available).unwrap();
    assert!(
        manager.reserve_payload(None, 1).is_err(),
        "worker-owned source buffers stay charged after cancellation"
    );
    drop(spare);
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while manager.inner.work.active.load(Ordering::Acquire) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let all = manager.reserve_payload(None, 65536 - used).unwrap();
    drop(all);
}
