#![forbid(unsafe_code)]

use std::{fs, path::Path};

use kiaro_secret_chat::binding_migration::{
    BindingOwnershipProof, MigrationSummary, migrate_chat_store_v1_to_v2,
    migrate_custody_store_v1_to_v2, migrate_ingress_store_v1_to_v2,
};
use kiaro_secret_chat::custody::{
    CustodyBackend, CustodyLimits, CustodyMode, CustodyRecord, CustodyStore,
};
use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::ingress::{
    CommitReplayStatus, DurableIngressController, EncryptedIngressStore, IngressLimits,
    IngressOrigin, IngressUpdate,
};
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, encrypt_frame, wrap_frame};
use kiaro_secret_chat::runtime_modes::{PersistentSecretChatManager, StrongSecretChatManager};
use kiaro_secret_chat::storage_backend::{FileStorage, PersistedChatRecord, SecretChatStore};
use serde_json::json;
use sha2::{Digest, Sha256};

const STORE_KEY: [u8; 32] = [0x32; 32];
const CHAT_KEY: [u8; 256] = [0x42; 256];
const TAG: [u8; 32] = [0xab; 32];
type TestResult = Result<(), Box<dyn std::error::Error>>;
type Migrate =
    fn(&Path, [u8; 32], &BindingOwnershipProof) -> Result<MigrationSummary, SecretChatError>;

fn legacy_binding(device_hash: i64) -> String {
    let mut hash = Sha256::new();
    hash.update(b"kiaro-grammers-binding-v1\0");
    hash.update(912_i64.to_le_bytes());
    hash.update(device_hash.to_le_bytes());
    hash.update(4_i32.to_le_bytes());
    hash.update(b"migration-test");
    hex::encode(hash.finalize())
}

fn proof() -> BindingOwnershipProof {
    BindingOwnershipProof {
        user_id: 912,
        dc_id: 4,
        generation: "migration-test".into(),
        auth_key_tag: TAG,
        legacy_v1_binding: legacy_binding(55),
    }
}

struct OfflineTransport(String);

impl SecretTransport for OfflineTransport {
    fn auth_identity(&self) -> &str {
        &self.0
    }

    fn send_encrypted(
        &self,
        _: i64,
        _: i64,
        _: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

fn chat_record(id: i32) -> Result<PersistedChatRecord, SecretChatError> {
    PersistedChatRecord::established(id, 101, 202, ChatRole::Acceptor, CHAT_KEY)
}

fn update(stream: &str, order: i64, payload: Vec<u8>) -> IngressUpdate {
    IngressUpdate {
        stream_id: stream.into(),
        order,
        provenance: IngressOrigin::VerifiedTransport,
        associated_chat_id: Some(7),
        payload,
    }
}

fn restore_ingress(
    path: &Path,
    binding: &str,
) -> Result<DurableIngressController, SecretChatError> {
    DurableIngressController::restore(
        binding,
        EncryptedIngressStore::open(path, STORE_KEY)?,
        IngressLimits::default(),
    )
}

fn custody_record(id: i64, binding: &str) -> Result<CustodyRecord, SecretChatError> {
    let mut record = CustodyRecord::new(
        id,
        binding,
        CustodyMode::Durable,
        CustodyBackend::EncryptedFile,
        CustodyLimits::default(),
    );
    record.push_payload(&[1, 2, 3])?;
    record.push_payload(&[4, 5])?;
    if id == 8 {
        record.pause("caller paused custody");
    }
    Ok(record)
}

fn assert_migration(
    migrate: Migrate,
    path: &Path,
    proof: &BindingOwnershipProof,
    count: usize,
) -> TestResult {
    let old = fs::read(path)?;
    let expected = proof.expected_v2_binding()?;
    assert_eq!(
        migrate(path, STORE_KEY, proof)?,
        MigrationSummary {
            migrated_records: count,
            from_binding: proof.legacy_v1_binding.clone(),
            to_binding: expected.clone(),
            already_migrated: false,
        }
    );
    let new = fs::read(path)?;
    assert_eq!(&old[..8], &new[..8]);
    assert_ne!(&old[8..20], &new[8..20]);
    assert_eq!(
        migrate(path, STORE_KEY, proof)?,
        MigrationSummary {
            migrated_records: 0,
            from_binding: proof.legacy_v1_binding.clone(),
            to_binding: expected,
            already_migrated: true,
        }
    );
    assert_eq!(fs::read(path)?, new);
    Ok(())
}

#[tokio::test]
async fn preserves_chat_records_receipts_keys_and_ingress_replay_state() -> TestResult {
    let dir = tempfile::tempdir()?;
    let chat_path = dir.path().join("chat state");
    let ingress_path = dir.path().join("ingress state");
    let proof = proof();
    let v1 = &proof.legacy_v1_binding;
    let v2 = proof.expected_v2_binding()?;
    let stream = "secret-chat/7";
    let frame = |order: i64| -> Result<Vec<u8>, SecretChatError> {
        let body = wrap_frame(
            &json!({"random_id": order, "ttl": 0, "message": format!("test {order}")}),
            46,
            0,
            order * 2 - 1,
        )?;
        encrypt_frame(&CHAT_KEY, &body, 0)
    };
    let first = update(stream, 1, frame(1)?);
    let pending = update(stream, 2, frame(2)?);
    let other_stream = update("other", 11, vec![7, 8, 9]);
    let mut ingress = restore_ingress(&ingress_path, v1)?;
    ingress.capture_and_commit(other_stream.clone())?;
    let store = FileStorage::open(&chat_path, STORE_KEY, v1)?;
    store.mark_transient_suspended(17)?;
    let manager = StrongSecretChatManager::new(
        PersistentSecretChatManager::open(
            OfflineTransport(v1.clone()),
            store,
            ManagerLimits::default(),
        )
        .await?,
        ingress,
    )?;
    manager.register_and_persist(chat_record(7)?).await?;
    manager.register_and_persist(chat_record(8)?).await?;
    manager.send_text(7, "retained outbox").await?;
    manager.ingest_before_ack(first.clone()).await?;
    manager.apply_delivery(7, 1).await?;
    assert!(manager.acknowledge_delivery(7, 1).await?);
    manager.ingest_before_ack(pending.clone()).await?;
    assert_eq!(
        manager.apply_delivery(7, 2).await?[0].text.as_deref(),
        Some("test 2")
    );
    drop(manager);

    let before = FileStorage::open(&chat_path, STORE_KEY, v1)?.load_all()?;
    assert_eq!(before.chats[&7].in_seq_no, 2);
    assert_eq!(before.chats[&7].out_seq_no, 1);
    assert_eq!(before.chats[&7].pending_outbox.len(), 1);
    assert_eq!(before.chats[&7].auth_key, CHAT_KEY);
    let mut expected = serde_json::to_value(&before)?;
    expected["account_binding"] = json!(v2);
    assert_migration(migrate_chat_store_v1_to_v2, &chat_path, &proof, 2)?;
    assert_migration(migrate_ingress_store_v1_to_v2, &ingress_path, &proof, 2)?;
    assert!(FileStorage::open(&chat_path, STORE_KEY, v1).is_err());
    assert!(restore_ingress(&ingress_path, v1).is_err());
    let store = FileStorage::open(&chat_path, STORE_KEY, &v2)?;
    assert!(serde_json::to_value(store.load_all()?)? == expected);

    let mut ingress = restore_ingress(&ingress_path, &v2)?;
    assert_eq!(ingress.owner_auth_id(), v2);
    assert_eq!(ingress.applied_cursor(stream)?, 1);
    for update in [pending, other_stream] {
        let records = ingress.pending_in_order(&update.stream_id);
        assert_eq!(records.len(), 1);
        let restored = &records[0];
        assert_eq!(restored.stream_id, update.stream_id);
        assert_eq!(restored.order, update.order);
        assert_eq!(restored.provenance, update.provenance);
        assert_eq!(restored.associated_chat_id, update.associated_chat_id);
        assert!(restored.validated);
        assert_eq!(restored.status, CommitReplayStatus::Replayed);
        assert_eq!(restored.payload, update.payload);
    }
    assert_eq!(
        ingress.capture_and_commit(first)?.status,
        CommitReplayStatus::DuplicateIgnored
    );
    let manager = StrongSecretChatManager::new(
        PersistentSecretChatManager::open(OfflineTransport(v2), store, ManagerLimits::default())
            .await?,
        ingress,
    )?;
    let reopened = manager.apply_delivery(7, 2).await?;
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].text.as_deref(), Some("test 2"));
    assert!(manager.acknowledge_delivery(7, 2).await?);
    Ok(())
}

#[test]
fn rebinds_every_custody_record_without_losing_payload_or_pause_state() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("custody state");
    let proof = proof();
    let v1 = &proof.legacy_v1_binding;
    let v2 = proof.expected_v2_binding()?;
    let mut store = CustodyStore::open_durable(&path, STORE_KEY)?;
    let originals = [custody_record(7, v1)?, custody_record(8, v1)?];
    for record in &originals {
        store.save_record(record)?;
    }
    drop(store);

    assert_migration(migrate_custody_store_v1_to_v2, &path, &proof, 2)?;
    let store = CustodyStore::open_durable(&path, STORE_KEY)?;
    assert!(store.load_record(7, v1).is_err());
    for record in originals {
        let actual = store.load_record(record.chat_id(), &v2)?;
        let mut expected = serde_json::to_value(record)?;
        expected["owner_auth_id"] = json!(v2);
        assert!(serde_json::to_value(actual)? == expected);
    }
    Ok(())
}

#[test]
fn every_store_refuses_bad_proof_wrong_key_and_corrupt_ciphertext() -> TestResult {
    let dir = tempfile::tempdir()?;
    let good = proof();
    let v1 = &good.legacy_v1_binding;
    let chat = dir.path().join("chat");
    assert!(matches!(
        migrate_chat_store_v1_to_v2(&chat, STORE_KEY, &good),
        Err(SecretChatError::StoreCorrupt { .. })
    ));
    assert!(!chat.exists());
    let custody = dir.path().join("custody");
    let ingress = dir.path().join("ingress");
    FileStorage::open(&chat, STORE_KEY, v1)?.save(&chat_record(7)?)?;
    CustodyStore::open_durable(&custody, STORE_KEY)?.save_record(&custody_record(7, v1)?)?;
    restore_ingress(&ingress, v1)?.capture_and_commit(update("test", 1, vec![1]))?;
    let mut wrong_legacy = proof();
    wrong_legacy.legacy_v1_binding = legacy_binding(56);
    let mut bad_user = proof();
    bad_user.user_id = 0;
    let mut bad_dc = proof();
    bad_dc.dc_id = -1;
    let mut bad_generation = proof();
    bad_generation.generation.clear();
    let mut empty_legacy = proof();
    empty_legacy.legacy_v1_binding.clear();
    let mut same_binding = proof();
    same_binding.legacy_v1_binding = good.expected_v2_binding()?;

    for (path, migrate) in [
        (&chat, migrate_chat_store_v1_to_v2 as Migrate),
        (&custody, migrate_custody_store_v1_to_v2 as Migrate),
        (&ingress, migrate_ingress_store_v1_to_v2 as Migrate),
    ] {
        let old = fs::read(path)?;
        for (key, proof) in [
            ([0; 32], &good),
            (STORE_KEY, &wrong_legacy),
            (STORE_KEY, &bad_user),
            (STORE_KEY, &bad_dc),
            (STORE_KEY, &bad_generation),
            (STORE_KEY, &empty_legacy),
            (STORE_KEY, &same_binding),
        ] {
            assert!(matches!(
                migrate(path, key, proof),
                Err(SecretChatError::StoreCorrupt { .. })
            ));
            assert_eq!(fs::read(path)?, old);
        }
        let mut corrupt = old;
        corrupt[20] ^= 1;
        fs::write(path, &corrupt)?;
        assert!(matches!(
            migrate(path, STORE_KEY, &good),
            Err(SecretChatError::StoreCorrupt { .. })
        ));
        assert_eq!(fs::read(path)?, corrupt);
    }
    Ok(())
}

#[test]
fn current_binding_matches_transport_bytes_and_proof_debug_redacts_tag() -> TestResult {
    let proof = proof();
    let mut hash = Sha256::new();
    hash.update(b"kiaro-grammers-binding-v2\0");
    hash.update(912_i64.to_le_bytes());
    hash.update(TAG);
    hash.update(4_i32.to_le_bytes());
    hash.update(b"migration-test");
    assert_eq!(proof.expected_v2_binding()?, hex::encode(hash.finalize()));
    for debug in [format!("{proof:?}"), format!("{proof:#?}")] {
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("171"));
        assert!(!debug.contains(&hex::encode(TAG)));
        assert!(!debug.contains(&proof.legacy_v1_binding));
    }
    Ok(())
}
