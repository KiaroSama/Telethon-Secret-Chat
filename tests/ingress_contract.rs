use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::ingress::{
    CommitReplayStatus, DurableIngressController, DurableIngressRecord, EncryptedIngressStore,
    IngressLimits, IngressOrigin, IngressUpdate,
};

fn test_store_key() -> [u8; 32] {
    [0x33u8; 32]
}

fn sample_update(stream_id: &str, qts: i64, chat_id: i64, payload: &[u8]) -> IngressUpdate {
    IngressUpdate {
        stream_id: stream_id.to_owned(),
        order: qts,
        provenance: IngressOrigin::LiveSocket,
        associated_chat_id: Some(chat_id),
        payload: payload.to_vec(),
    }
}

#[test]
fn durable_ingress_preserves_order_deduplicates_and_replays_across_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("ingress.enc");
    let store = EncryptedIngressStore::open(&store_path, test_store_key()).expect("open store");

    let mut controller =
        DurableIngressController::new("auth-owner-1", store, IngressLimits::default());

    // Direct, repeated, and out-of-order inputs.
    let first = controller
        .capture_and_commit(sample_update("account-main", 41, 7, b"frame-41"))
        .expect("commit qts 41");
    assert_eq!(first.status, CommitReplayStatus::Committed);
    assert_eq!(first.order, 41);

    // Exact duplicate must not redeliver.
    let dup = controller
        .capture_and_commit(sample_update("account-main", 41, 7, b"frame-41"))
        .expect("duplicate qts 41");
    assert_eq!(dup.status, CommitReplayStatus::DuplicateIgnored);

    // Out-of-order (qts 43 before qts 42) is persisted, then ordered drain delivers 41, 42, 43.
    let future = controller
        .capture_and_commit(sample_update("account-main", 43, 7, b"frame-43"))
        .expect("commit qts 43");
    assert_eq!(future.status, CommitReplayStatus::Committed);

    let gap_fill = controller
        .capture_and_commit(sample_update("account-main", 42, 7, b"frame-42"))
        .expect("commit qts 42");
    assert_eq!(gap_fill.status, CommitReplayStatus::Committed);

    // Restart from disk with the same authorization identity -> replays committed records in strict order without duplicates.
    drop(controller);
    let reopened = EncryptedIngressStore::open(&store_path, test_store_key()).expect("reopen");
    let mut restored =
        DurableIngressController::restore("auth-owner-1", reopened, IngressLimits::default())
            .expect("restore");

    let replayed: Vec<DurableIngressRecord> = restored
        .drain_in_order("account-main")
        .expect("durable drain");
    let orders: Vec<i64> = replayed.iter().map(|r| r.order).collect();
    assert_eq!(orders, vec![41, 42, 43]);
    assert!(
        replayed
            .iter()
            .all(|r| r.status == CommitReplayStatus::Replayed)
    );
    assert!(
        restored
            .drain_in_order("account-main")
            .expect("empty drain")
            .is_empty()
    );
}

#[test]
fn unavailable_or_tampered_storage_and_wrong_owner_refuse_without_advancing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("ingress.enc");
    let store = EncryptedIngressStore::open(&store_path, test_store_key()).expect("open");
    let mut controller =
        DurableIngressController::new("auth-owner-1", store, IngressLimits::default());
    controller
        .capture_and_commit(sample_update("account-main", 10, 7, b"frame-10"))
        .expect("commit 10");

    // Wrong authorization identity must be refused on restore (no cross-account adoption).
    let reopened_wrong_owner =
        EncryptedIngressStore::open(&store_path, test_store_key()).expect("reopen");
    let wrong_owner_err = DurableIngressController::restore(
        "auth-owner-2",
        reopened_wrong_owner,
        IngressLimits::default(),
    )
    .unwrap_err();
    assert!(matches!(
        wrong_owner_err,
        SecretChatError::IngressRefused { .. }
    ));

    // Tampered ciphertext on disk must fail closed on restore (AEAD tag mismatch).
    let mut raw = std::fs::read(&store_path).expect("read ciphertext");
    let last = raw.len() - 1;
    raw[last] ^= 0xff;
    std::fs::write(&store_path, raw).expect("write tampered");

    let tampered_store = EncryptedIngressStore::open(&store_path, test_store_key()).expect("open");
    let tamper_err =
        DurableIngressController::restore("auth-owner-1", tampered_store, IngressLimits::default())
            .unwrap_err();
    assert!(matches!(
        tamper_err,
        SecretChatError::IngressRefused { .. } | SecretChatError::StoreCorrupt { .. }
    ));

    // Unavailable storage (path points to an existing directory) refuses without advancing frontier.
    let bad_store = EncryptedIngressStore::open(dir.path(), test_store_key()).expect("open dir");
    let mut failing =
        DurableIngressController::new("auth-owner-1", bad_store, IngressLimits::default());
    let write_err = failing
        .capture_and_commit(sample_update("account-main", 1, 7, b"frame-1"))
        .unwrap_err();
    assert!(matches!(write_err, SecretChatError::IngressRefused { .. }));
    assert!(
        failing
            .drain_in_order("account-main")
            .expect("empty drain")
            .is_empty()
    );
}

#[tokio::test]
async fn stalled_and_cancelled_ingress_requests_do_not_advance_or_lose_prior_commits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("ingress.enc");
    let store = EncryptedIngressStore::open(&store_path, test_store_key()).expect("open");
    let controller = Arc::new(tokio::sync::Mutex::new(DurableIngressController::new(
        "auth-owner-1",
        store,
        IngressLimits::default(),
    )));

    // Commit record 1 cleanly first.
    {
        let mut guard = controller.lock().await;
        guard
            .capture_and_commit(sample_update("account-main", 1, 9, b"committed-1"))
            .expect("commit 1");
    }

    // Spawn a stalled request for record 2 and cancel it before adoption completes.
    let entered = Arc::new(AtomicBool::new(false));
    let entered_clone = Arc::clone(&entered);
    let ctrl_clone = Arc::clone(&controller);
    let task = tokio::spawn(async move {
        let mut guard = ctrl_clone.lock().await;
        guard
            .capture_with_barrier(
                sample_update("account-main", 2, 9, b"stalled-2"),
                || async {
                    entered_clone.store(true, Ordering::SeqCst);
                    std::future::pending::<Result<(), SecretChatError>>().await
                },
            )
            .await
    });

    // Wait until the stalled barrier is entered, then cancel it.
    tokio::time::timeout(Duration::from_secs(1), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("barrier never entered");
    task.abort();
    let _ = task.await;

    // Frontier must still hold only record 1; record 2 was never committed or adopted.
    let mut guard = controller.lock().await;
    let drained = guard.drain_in_order("account-main").expect("durable drain");
    let orders: Vec<i64> = drained.iter().map(|r| r.order).collect();
    assert_eq!(orders, vec![1]);
}
