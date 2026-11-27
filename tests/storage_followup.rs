use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use kiaro_secret_chat::custody::{
    CustodyBackend, CustodyLimits, CustodyMode, CustodyRecord, CustodyStore,
};
use kiaro_secret_chat::ingress::{
    CommitReplayStatus, DurableIngressController, EncryptedIngressStore, IngressLimits,
    IngressOrigin, IngressUpdate,
};
use serde_json::json;

const KEY: [u8; 32] = [0x39; 32];

fn record(chat_id: i64, owner: &str) -> CustodyRecord {
    let mut record = CustodyRecord::new(
        chat_id,
        owner,
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        CustodyLimits::default(),
    );
    record.push_payload(b"retained obligation").unwrap();
    record
}

fn update(order: i64) -> IngressUpdate {
    IngressUpdate {
        stream_id: "main".into(),
        order,
        provenance: IngressOrigin::LiveSocket,
        associated_chat_id: Some(7),
        payload: vec![1, 2, 3],
    }
}

fn restore(
    path: &Path,
) -> Result<DurableIngressController, kiaro_secret_chat::errors::SecretChatError> {
    DurableIngressController::restore(
        "owner",
        EncryptedIngressStore::open(path, KEY).unwrap(),
        IngressLimits::default(),
    )
}

fn write_authenticated_ingress(path: &Path, plaintext: &[u8]) {
    let nonce = [0x41; 12];
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(KEY));
    let ciphertext = cipher.encrypt(&Nonce::from(nonce), plaintext).unwrap();
    let mut envelope = b"KSCING01".to_vec();
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    std::fs::write(path, envelope).unwrap();
}

fn pending_record(order: i64) -> String {
    json!({
        "stream_id":"main", "order":order, "provenance":"LiveSocket",
        "validated":true, "status":"Committed", "associated_chat_id":7,
        "payload":[1,2,3]
    })
    .to_string()
}

#[test]
fn ingress_restore_rejects_duplicate_replay_stream_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    write_authenticated_ingress(
        &path,
        br#"{"owner_auth_id":"owner","seen_orders":{"main":[1],"main":[2]},"pending_by_stream":{}}"#,
    );
    assert!(
        restore(&path).is_err(),
        "restore discarded a replay identity"
    );
}

#[test]
fn ingress_restore_rejects_duplicate_replay_order_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    write_authenticated_ingress(
        &path,
        br#"{"owner_auth_id":"owner","seen_orders":{"main":[1,1]},"pending_by_stream":{}}"#,
    );
    assert!(
        restore(&path).is_err(),
        "restore normalized an ambiguous replay set"
    );
}

#[test]
fn ingress_restore_rejects_duplicate_pending_stream_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let first = pending_record(1);
    let second = pending_record(2);
    let plaintext = format!(
        "{{\"owner_auth_id\":\"owner\",\"seen_orders\":{{\"main\":[1,2]}},\"pending_by_stream\":{{\"main\":{{\"1\":{first}}},\"main\":{{\"2\":{second}}}}}}}"
    );
    write_authenticated_ingress(&path, plaintext.as_bytes());
    assert!(restore(&path).is_err(), "restore discarded an owed record");
}

#[test]
fn ingress_restore_rejects_duplicate_pending_order_keys() {
    let dir = tempfile::tempdir().unwrap();
    for repeated_key in ["1", "01"] {
        let path = dir.path().join(format!("ingress-{repeated_key}.enc"));
        let pending = pending_record(1);
        let plaintext = format!(
            "{{\"owner_auth_id\":\"owner\",\"seen_orders\":{{\"main\":[1]}},\"pending_by_stream\":{{\"main\":{{\"1\":{pending},\"{repeated_key}\":{pending}}}}}}}"
        );
        write_authenticated_ingress(&path, plaintext.as_bytes());
        assert!(
            restore(&path).is_err(),
            "restore normalized duplicate order key {repeated_key}"
        );
    }
}

#[test]
fn ingress_restore_rejects_untracked_empty_pending_streams() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let streams: serde_json::Map<String, serde_json::Value> = (0..4096)
        .map(|index| (format!("untracked-{index}"), json!({})))
        .collect();
    let plaintext = json!({
        "owner_auth_id":"owner", "seen_orders":{}, "pending_by_stream":streams
    });
    write_authenticated_ingress(&path, &serde_json::to_vec(&plaintext).unwrap());
    assert!(
        restore(&path).is_err(),
        "restore accepted untracked streams that make the next commit exceed the restore ceiling"
    );
}

#[test]
fn valid_stream_budget_boundary_remains_reopenable_after_capture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let streams: serde_json::Map<String, serde_json::Value> = (0..4096)
        .map(|index| (format!("tracked-{index}"), json!([1])))
        .collect();
    let plaintext = json!({
        "owner_auth_id":"owner", "seen_orders":streams, "pending_by_stream":{}
    });
    write_authenticated_ingress(&path, &serde_json::to_vec(&plaintext).unwrap());
    let mut controller = restore(&path).expect("restore the valid stream boundary");
    let mut next = update(2);
    next.stream_id = "tracked-0".into();
    controller.capture_and_commit(next.clone()).unwrap();
    drop(controller);

    let reopened = restore(&path).expect("reopen the successfully committed snapshot");
    let pending = reopened.pending_in_order(&next.stream_id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stream_id, next.stream_id);
    assert_eq!(pending[0].order, next.order);
    assert_eq!(pending[0].payload, next.payload);
    assert_eq!(pending[0].associated_chat_id, next.associated_chat_id);
    assert_eq!(pending[0].status, CommitReplayStatus::Replayed);
}

#[test]
fn empty_ingress_drain_refuses_a_stale_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut writer = restore(&path).unwrap();
    writer.capture_and_commit(update(1)).unwrap();
    writer.acknowledge("main", 1).unwrap();
    let mut stale = restore(&path).unwrap();
    writer.capture_and_commit(update(2)).unwrap();
    let latest = std::fs::read(&path).unwrap();
    assert!(
        stale.drain_in_order("main").is_err(),
        "destructive drain reported an empty stream without checking its stale revision"
    );
    assert_eq!(std::fs::read(&path).unwrap(), latest);
    assert_eq!(restore(&path).unwrap().pending_in_order("main").len(), 1);
}

#[test]
fn custody_stale_owner_cannot_delete_a_recreated_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let mut stale = CustodyStore::open_durable(&path, KEY).unwrap();
    stale.save_record(&record(7, "owner-a")).unwrap();
    let mut current = CustodyStore::open_durable(&path, KEY).unwrap();
    current.load_record(7, "owner-a").unwrap();
    current.cleanup_owned_files().unwrap();
    let mut replacement = CustodyStore::open_durable(&path, KEY).unwrap();
    replacement.save_record(&record(8, "owner-b")).unwrap();
    let latest = std::fs::read(&path).unwrap();
    assert!(
        stale.cleanup_owned_files().is_err(),
        "a handle previously used by one authorization deleted another owner's custody"
    );
    assert_eq!(std::fs::read(&path).unwrap(), latest);
}

#[test]
fn custody_owner_binding_survives_loading_and_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let mut writer = CustodyStore::open_durable(&path, KEY).unwrap();
    writer.save_record(&record(7, "owner-a")).unwrap();
    let reader = CustodyStore::open_durable(&path, KEY).unwrap();
    reader.load_record(7, "owner-a").unwrap();
    writer.cleanup_owned_files().unwrap();
    let mut replacement = CustodyStore::open_durable(&path, KEY).unwrap();
    replacement.save_record(&record(8, "owner-b")).unwrap();
    assert!(
        reader.load_record(8, "owner-b").is_err(),
        "a loaded handle silently changed its authorization binding"
    );
    replacement.cleanup_owned_files().unwrap();
    assert!(
        writer.save_record(&record(9, "owner-b")).is_err(),
        "cleanup silently reset a handle's authorization binding"
    );
}

#[test]
fn an_authenticated_store_cannot_be_reused_as_another_stores_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let sidecar_path = dir.path().join("custody.enc.lock");
    let mut existing = CustodyStore::open_durable(&sidecar_path, KEY).unwrap();
    existing.save_record(&record(7, "owner")).unwrap();
    let latest = std::fs::read(&sidecar_path).unwrap();
    let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
    assert!(
        store.save_record(&record(8, "owner")).is_err(),
        "another store's replaceable data file was accepted as a stable lock sidecar"
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read(&sidecar_path).unwrap(), latest);
}
