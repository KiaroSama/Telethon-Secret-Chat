use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use kiaro_secret_chat::custody::{
    CustodyBackend, CustodyLimits, CustodyMode, CustodyRecord, CustodyStore,
};
use kiaro_secret_chat::ingress::{
    CommitReplayStatus, DurableIngressController, DurableIngressRecord, EncryptedIngressStore,
    IngressLimits, IngressOrigin, IngressUpdate,
};
use serde_json::{Value, json};

const KEY: [u8; 32] = [0x72; 32];

fn custody(chat_id: i64, owner: &str) -> CustodyRecord {
    CustodyRecord::new(
        chat_id,
        owner,
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        CustodyLimits {
            max_retained_messages: 2,
            max_stored_bytes: 16,
        },
    )
}

fn update(order: i64) -> IngressUpdate {
    IngressUpdate {
        stream_id: "main".into(),
        order,
        provenance: IngressOrigin::LiveSocket,
        associated_chat_id: Some(7),
        payload: vec![order as u8],
    }
}

fn ingress(path: &Path, owner: &str, limit: usize) -> DurableIngressController {
    DurableIngressController::new(
        owner,
        EncryptedIngressStore::open(path, KEY).unwrap(),
        IngressLimits {
            max_pending_records: limit,
            max_payload_bytes: 64,
        },
    )
}

fn write_authenticated_json(path: &Path, magic: &[u8; 8], state: &Value) {
    let nonce = [0x12; 12];
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(KEY));
    let plaintext = serde_json::to_vec(state).unwrap();
    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), plaintext.as_slice())
        .unwrap();
    let mut raw = magic.to_vec();
    raw.extend_from_slice(&nonce);
    raw.extend_from_slice(&ciphertext);
    std::fs::write(path, raw).unwrap();
}

#[test]
fn custody_debug_exposes_counts_without_retained_payload_or_pause_text() {
    let mut record = custody(7, "secret");
    record.push_payload(b"secret").unwrap();
    record.pause("secret");
    let diagnostic = format!("{record:?}");
    assert!(!diagnostic.contains("secret"));
    assert!(!diagnostic.contains(&format!("{:?}", b"secret")));
    assert!(diagnostic.contains("retained_count: 1"));
    assert!(diagnostic.contains("retained_bytes: 6"));
}

#[test]
fn ingress_debug_exposes_payload_length_without_payload_or_stream_text() {
    let update = IngressUpdate {
        stream_id: "secret".into(),
        payload: b"secret".to_vec(),
        ..update(1)
    };
    let record = DurableIngressRecord {
        stream_id: update.stream_id.clone(),
        order: update.order,
        provenance: update.provenance,
        validated: true,
        status: CommitReplayStatus::Committed,
        associated_chat_id: update.associated_chat_id,
        payload: update.payload.clone(),
    };
    for diagnostic in [format!("{update:?}"), format!("{record:?}")] {
        assert!(!diagnostic.contains("secret"));
        assert!(!diagnostic.contains(&format!("{:?}", b"secret")));
        assert!(diagnostic.contains("payload_bytes: 6"));
    }
}

#[test]
fn custody_capacity_preserves_owed_payloads_and_pauses() {
    let mut record = custody(7, "owner");
    record.push_payload(b"first").unwrap();
    record.push_payload(b"second").unwrap();
    let before = serde_json::to_value(&record).unwrap()["retained_payloads"].clone();
    assert!(record.push_payload(b"third").is_err());
    assert!(record.is_paused());
    assert_eq!(
        serde_json::to_value(&record).unwrap()["retained_payloads"],
        before
    );
    assert_eq!(record.retained_bytes(), 11);
}

#[test]
fn custody_byte_capacity_preserves_owed_payloads() {
    let mut record = custody(7, "owner");
    record.push_payload(b"0123456789").unwrap();
    assert!(record.push_payload(b"0123456789").is_err());
    assert_eq!(record.retained_count(), 1);
    assert_eq!(record.retained_bytes(), 10);
}

#[test]
fn custody_rejects_inconsistent_deserialized_accounting() {
    let mut value = serde_json::to_value(custody(7, "owner")).unwrap();
    value["retained_bytes"] = json!(usize::MAX);
    assert!(serde_json::from_value::<CustodyRecord>(value).is_err());
}

#[test]
fn custody_refuses_cross_authorization_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
    store.save_record(&custody(7, "owner-a")).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(store.save_record(&custody(7, "owner-b")).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn custody_refuses_persisting_memory_only_and_transient_payloads() {
    let dir = tempfile::tempdir().unwrap();
    for (index, mode, backend) in [
        (0, CustodyMode::Transient, CustodyBackend::EncryptedFile),
        (1, CustodyMode::Durable, CustodyBackend::MemoryOnly),
    ] {
        let path = dir.path().join(format!("{index}.enc"));
        let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
        let mut record = CustodyRecord::new(7, "owner", mode, backend, CustodyLimits::default());
        record.push_payload(b"volatile").unwrap();
        assert!(store.save_record(&record).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn custody_staging_and_cleanup_preserve_unowned_tmp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let unowned = dir.path().join("custody.enc.tmp");
    std::fs::write(&unowned, b"unrelated file").unwrap();
    let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
    store.save_record(&custody(7, "owner")).unwrap();
    assert_eq!(std::fs::read(&unowned).unwrap(), b"unrelated file");
    store.cleanup_owned_files().unwrap();
    assert_eq!(std::fs::read(&unowned).unwrap(), b"unrelated file");
}

#[test]
fn custody_cleanup_does_not_delete_an_unauthenticated_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unrelated.enc");
    std::fs::write(&path, b"unrelated data").unwrap();
    let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
    assert!(store.cleanup_owned_files().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated data");
}

#[test]
fn custody_rejects_authenticated_but_inconsistent_records_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let mut record = serde_json::to_value(custody(7, "owner")).unwrap();
    record["retained_bytes"] = json!(9);
    write_authenticated_json(&path, b"KSCCUS01", &json!([record]));
    let store = CustodyStore::open_durable(&path, KEY).unwrap();
    assert!(store.load_record(7, "owner").is_err());
}

#[test]
fn ingress_refuses_relative_paths_with_explicit_parent() {
    assert!(EncryptedIngressStore::open("./relative-ingress.enc", KEY).is_err());
}

#[test]
fn ingress_duplicate_is_accepted_when_pending_queue_is_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut controller = ingress(&path, "owner", 1);
    controller.capture_and_commit(update(1)).unwrap();
    assert_eq!(
        controller.capture_and_commit(update(1)).unwrap().status,
        CommitReplayStatus::DuplicateIgnored
    );
}

#[test]
fn ingress_new_controller_cannot_overwrite_existing_custody() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut first = ingress(&path, "owner-a", 8);
    first.capture_and_commit(update(1)).unwrap();
    drop(first);
    let before = std::fs::read(&path).unwrap();
    let mut second = ingress(&path, "owner-b", 8);
    assert!(second.capture_and_commit(update(2)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn ingress_stale_restored_writer_cannot_erase_a_new_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut first = ingress(&path, "owner", 8);
    first.capture_and_commit(update(1)).unwrap();
    let mut second = DurableIngressController::restore(
        "owner",
        EncryptedIngressStore::open(&path, KEY).unwrap(),
        IngressLimits::default(),
    )
    .unwrap();
    first.capture_and_commit(update(2)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(second.capture_and_commit(update(3)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn active_store_lock_prevents_an_independent_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custody.enc");
    let mut store = CustodyStore::open_durable(&path, KEY).unwrap();
    store.save_record(&custody(7, "owner")).unwrap();
    let before = std::fs::read(&path).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.path().join("custody.enc.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(store.save_record(&custody(8, "owner")).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn invalid_zero_limits_and_empty_owner_fail_closed() {
    let mut record = CustodyRecord::new(
        7,
        "",
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        CustodyLimits::default(),
    );
    assert!(record.push_payload(b"payload").is_err());
    let mut zero = CustodyRecord::new(
        7,
        "owner",
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        CustodyLimits {
            max_retained_messages: 1,
            max_stored_bytes: 0,
        },
    );
    assert!(zero.push_payload(b"").is_err());
    let dir = tempfile::tempdir().unwrap();
    let mut controller = ingress(&dir.path().join("ingress.enc"), "", 8);
    assert!(controller.capture_and_commit(update(1)).is_err());
}

#[test]
fn ingress_refuses_conflicting_duplicate_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    let mut conflicting = update(1);
    conflicting.payload.push(99);
    assert!(controller.capture_and_commit(conflicting).is_err());
}

#[test]
fn ingress_drain_failure_preserves_pending_custody() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let backup = dir.path().join("saved.enc");
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(controller.drain_in_order("main").is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    assert_eq!(controller.drain_in_order("main").unwrap().len(), 1);
}

#[test]
fn ingress_polling_retains_obligations_until_explicit_durable_acknowledgment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    controller.capture_and_commit(update(2)).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut detached = controller.pending_in_order("main");
    detached[0].payload.push(99);
    assert_eq!(controller.pending_in_order("main")[0].payload, vec![1]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    drop(controller);
    let mut restored = DurableIngressController::restore(
        "owner",
        EncryptedIngressStore::open(&path, KEY).unwrap(),
        IngressLimits::default(),
    )
    .unwrap();
    assert_eq!(restored.pending_in_order("main").len(), 2);
    assert!(restored.acknowledge("main", 1).unwrap());
    assert!(!restored.acknowledge("main", 1).unwrap());
    assert!(restored.acknowledge("main", 3).is_err());
    assert_eq!(
        restored.capture_and_commit(update(1)).unwrap().status,
        CommitReplayStatus::DuplicateIgnored
    );
    drop(restored);
    let restored = DurableIngressController::restore(
        "owner",
        EncryptedIngressStore::open(&path, KEY).unwrap(),
        IngressLimits::default(),
    )
    .unwrap();
    assert_eq!(
        restored
            .pending_in_order("main")
            .iter()
            .map(|record| record.order)
            .collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
fn ingress_failed_acknowledgment_keeps_the_obligation_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let backup = dir.path().join("saved.enc");
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(controller.acknowledge("main", 1).is_err());
    assert_eq!(controller.pending_in_order("main").len(), 1);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    assert!(controller.acknowledge("main", 1).unwrap());
    assert!(controller.pending_in_order("main").is_empty());
}

#[test]
fn ingress_restore_rejects_pending_state_over_configured_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    controller.capture_and_commit(update(2)).unwrap();
    drop(controller);
    assert!(
        DurableIngressController::restore(
            "owner",
            EncryptedIngressStore::open(&path, KEY).unwrap(),
            IngressLimits {
                max_pending_records: 1,
                max_payload_bytes: 64
            }
        )
        .is_err()
    );
}

#[test]
fn ingress_restore_rejects_inconsistent_authenticated_state() {
    let dir = tempfile::tempdir().unwrap();
    for (index, field, value) in [
        (0, "stream_id", json!("another")),
        (1, "order", json!(9)),
        (2, "validated", json!(false)),
        (3, "status", json!("DuplicateIgnored")),
    ] {
        let path = dir.path().join(format!("{index}.enc"));
        let mut record = json!({"stream_id":"main", "order":1, "provenance":"LiveSocket",
            "validated":true, "status":"Committed", "associated_chat_id":7, "payload":[1]});
        record[field] = value;
        write_authenticated_json(
            &path,
            b"KSCING01",
            &json!({
                "owner_auth_id":"owner", "seen_orders":{"main":[1]},
                "pending_by_stream":{"main":{"1":record}}
            }),
        );
        assert!(
            DurableIngressController::restore(
                "owner",
                EncryptedIngressStore::open(&path, KEY).unwrap(),
                IngressLimits::default()
            )
            .is_err(),
            "accepted inconsistent {field}"
        );
    }
}

#[test]
fn ingress_staging_preserves_unowned_tmp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.enc");
    let unowned = dir.path().join("ingress.enc.tmp");
    std::fs::write(&unowned, b"unrelated file").unwrap();
    let mut controller = ingress(&path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    assert_eq!(std::fs::read(&unowned).unwrap(), b"unrelated file");
}

#[cfg(unix)]
#[test]
fn staging_symlinks_never_overwrite_unrelated_files() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let unrelated = dir.path().join("unrelated");
    std::fs::write(&unrelated, b"keep").unwrap();
    let custody_path = dir.path().join("custody.enc");
    symlink(&unrelated, dir.path().join("custody.enc.tmp")).unwrap();
    let mut store = CustodyStore::open_durable(&custody_path, KEY).unwrap();
    store.save_record(&custody(7, "owner")).unwrap();
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    let ingress_path = dir.path().join("ingress.enc");
    symlink(&unrelated, dir.path().join("ingress.enc.tmp")).unwrap();
    let mut controller = ingress(&ingress_path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn persisted_encrypted_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let custody_path = dir.path().join("custody.enc");
    let mut store = CustodyStore::open_durable(&custody_path, KEY).unwrap();
    store.save_record(&custody(7, "owner")).unwrap();
    std::fs::set_permissions(&custody_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    store.save_record(&custody(8, "owner")).unwrap();
    assert_eq!(
        std::fs::metadata(&custody_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let ingress_path = dir.path().join("ingress.enc");
    let mut controller = ingress(&ingress_path, "owner", 8);
    controller.capture_and_commit(update(1)).unwrap();
    assert_eq!(
        std::fs::metadata(&ingress_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
