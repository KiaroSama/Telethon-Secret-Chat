#![allow(clippy::unwrap_used)]
#[path = "support_manager/bounded.rs"]
mod bounded;
#[path = "support_manager/mod.rs"]
mod support;
use bounded::*;
use kiaro_secret_chat::{
    custody::CustodyStorage,
    error::Error,
    manager::{Options, RpcFailure, SecretChatManager, SendFileOptions},
    media::FileSource,
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::*;
async fn soon<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
#[tokio::test]
async fn uncertain_send_and_retry_keep_original_public_identity() {
    let (a, b, w, id) = pair(TestStorage::default(), limits(), Options::default()).await;
    w.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending {
        chat_id,
        random_id,
        cause,
    } = a
        .send_message(id, "uncertain fixture", None, None)
        .await
        .unwrap_err()
    else {
        panic!("expected pending identity");
    };
    assert_eq!(chat_id, id);
    assert_eq!(cause.name(), "TransportError");
    let first = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert_eq!(first.random_id, random_id);
    w.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let Error::SendPending {
        random_id: again, ..
    } = a.retry_pending(id).await.unwrap_err()
    else {
        panic!("expected retry identity");
    };
    let retried = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert_eq!(again, random_id);
    assert_eq!(retried.random_id, random_id);
    assert_eq!(retried.frame, first.frame);
    w.lock().unwrap().fail_send = Some(RpcFailure::Cancelled);
    let Error::SendPending {
        random_id: cancelled,
        cause,
        ..
    } = a.retry_pending(id).await.unwrap_err()
    else {
        panic!("expected cancelled retry identity");
    };
    assert_eq!(cancelled, random_id);
    assert_eq!(cause.name(), "CancelledError");
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn record_capacity_refuses_upload_before_network() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), limits(), Options::default()).await;
    a.stop().await.unwrap();
    let mut quota = limits();
    quota.records = 2;
    let resumed = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        CustodyStorage::new(store, "11".repeat(32), quota).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    resumed.start().await.unwrap();
    let before = w.lock().unwrap().uploads;
    assert!(
        resumed
            .send_file(
                id,
                FileSource::bytes(b"refused file".to_vec(), "file.bin").unwrap(),
                SendFileOptions::default()
            )
            .await
            .is_err()
    );
    assert_eq!(w.lock().unwrap().uploads, before);
    assert!(w.lock().unwrap().discards.is_empty());
    resumed.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn concurrent_create_reserves_chat_slot_before_dh_rpc() {
    let (transport, _) = Fake::pair();
    let w = Arc::clone(&transport.world);
    let gate = Arc::new(Gate::default());
    w.lock().unwrap().dh_gate = Some(Arc::clone(&gate));
    let mut quota = limits();
    quota.chats = 1;
    let a = SecretChatManager::new(
        transport,
        CustodyStorage::new(TestStorage::default(), "11".repeat(32), quota).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    let manager = a.clone();
    let task = tokio::spawn(async move { manager.create(peer(2000)).await });
    soon(gate.entered.notified()).await;
    assert!(a.create(peer(2001)).await.is_err());
    assert!(w.lock().unwrap().request_ids.is_empty());
    w.lock().unwrap().dh_gate = None;
    gate.release.notify_one();
    soon(task).await.unwrap().unwrap();
    assert_eq!(w.lock().unwrap().request_ids.len(), 1);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn failed_receive_commit_keeps_prior_receipt_and_blocks_resume() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), limits(), Options::default()).await;
    b.send_message(id, "retained prior", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    let before = a.pending_relay_deliveries(id).unwrap();
    store.failures.store(1, Ordering::Release);
    b.send_message(id, "failed commit", None, None)
        .await
        .unwrap();
    pump(&a, &b, &w).await;
    let after = a.pending_relay_deliveries(id).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].seq_no, before[0].seq_no);
    assert_eq!(after[0].message.body, before[0].message.body);
    assert!(!a.pause_status(id).unwrap().resumable);
    assert!(a.resume_chat(id).await.is_err());
    assert!(w.lock().unwrap().discards.is_empty());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn send_result_commit_failure_keeps_original_wire_for_restart() {
    let store = TestStorage::default();
    let (a, b, w, id) = pair(store.clone(), limits(), Options::default()).await;
    w.lock().unwrap().fail_store_after_send = Some(store.clone());
    let Error::SendPending {
        random_id, cause, ..
    } = a
        .send_message(id, "result fault fixture", None, None)
        .await
        .unwrap_err()
    else {
        panic!("expected retained original wire");
    };
    assert_eq!(cause.name(), "StorageError");
    let first = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    assert_eq!(first.random_id, random_id);
    a.stop().await.unwrap();
    let resumed = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        CustodyStorage::new(store, "11".repeat(32), limits()).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let sent_before_restart = w.lock().unwrap().sent.len();
    resumed.start().await.unwrap();
    let again = {
        let sent = w.lock().unwrap();
        assert!(
            sent.sent.len() > sent_before_restart,
            "restart must issue a new retry"
        );
        sent.sent[sent_before_restart].outgoing.clone()
    };
    assert_eq!(again.random_id, random_id);
    assert_eq!(again.frame, first.frame);
    resumed.stop().await.unwrap();
    b.stop().await.unwrap();
}
