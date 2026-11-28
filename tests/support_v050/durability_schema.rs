use super::*;
use kiaro_secret_chat::{
    autosave::AutoSave,
    custody::{CustodyLimits, CustodyStorage},
    enums,
    files::FileKey,
    media::MediaReference,
    types,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

fn duplicate_entry(map: &Value, key: &str, duplicate_key: &str) -> TestResult<String> {
    let value = serde_json::to_string(map.get(key).ok_or("missing map identity")?)?;
    let entry = format!("{}:{value}", serde_json::to_string(key)?);
    let duplicate = format!("{entry},{}:{value}", serde_json::to_string(duplicate_key)?);
    let encoded = serde_json::to_string(map)?;
    assert_eq!(encoded.matches(&entry).count(), 1);
    Ok(encoded.replacen(&entry, &duplicate, 1))
}

fn replace_map(document: &str, field: &str, map: &Value, replacement: &str) -> TestResult<String> {
    let name = serde_json::to_string(field)?;
    let entry = format!("{name}:{}", serde_json::to_string(map)?);
    assert_eq!(document.matches(&entry).count(), 1);
    Ok(document.replacen(&entry, &format!("{name}:{replacement}"), 1))
}

fn queued_state() -> TestResult<State> {
    fn peer(outbound: bool) -> TestResult<Protocol<MemoryStorage>> {
        let mut protocol = Protocol::open(MemoryStorage::default(), "11".repeat(32))?;
        protocol.start()?;
        let mut chat = Chat::for_role(7, 8, 9, outbound);
        chat.adopt_key(SharedKey::from_slice(&[0x57; 256])?)?;
        protocol.insert_chat(chat)?;
        Ok(protocol)
    }
    let mut receiver = peer(true)?;
    let mut sender = peer(false)?;
    receiver.send_message(7, "retained outgoing".into(), None, None)?;
    let ordinary = sender.send_message(7, "ordinary delivery".into(), None, None)?;
    receiver.receive(7, &ordinary.outgoing.frame, None, 124)?;
    let service = sender.send_action(7, enums::DecryptedMessageAction::Noop)?;
    receiver.receive(7, &service.outgoing.frame, None, 124)?;
    sender.send_message(7, "missing middle".into(), None, None)?;
    let later = sender.send_message(7, "retained gap".into(), None, None)?;
    receiver.receive(7, &later.outgoing.frame, None, 124)?;
    let mut state = State::empty("11".repeat(32))?;
    state.chats.insert(7, receiver.chat_record(7)?);
    state.validate()?;
    Ok(state)
}

#[test]
fn native_queue_maps_reject_duplicate_and_aliased_sequence_identities() -> TestResult {
    let directory = tempfile::tempdir()?;
    let state = queued_state()?;
    let value = serde_json::to_value(&state)?;
    let encoded = serde_json::to_string(&value)?;
    for field in [
        "outbox",
        "gaps",
        "deliveries",
        "service_deliveries",
        "gap_key_fingerprints",
    ] {
        let map = &value["chats"]["7"][field];
        let key = map
            .as_object()
            .and_then(|map| map.keys().next())
            .ok_or("missing queue identity")?;
        for duplicate in [key.clone(), format!("0{key}")] {
            let path = directory.path().join(format!("{field}-{duplicate}.json"));
            let mut store = FileStorage::open(&path)?;
            store.commit(&state)?;
            let restored = store.load()?.ok_or("missing valid queued state")?;
            assert!(serde_json::to_value(restored)? == value);
            drop(store);
            let corrupt = replace_map(
                &encoded,
                field,
                map,
                &duplicate_entry(map, key, &duplicate)?,
            )?;
            std::fs::write(&path, &corrupt)?;
            assert!(
                FileStorage::open(&path)?.load().is_err(),
                "accepted duplicate {field} identity"
            );
            assert_eq!(std::fs::read(&path)?, corrupt.as_bytes());
        }
    }
    Ok(())
}

#[test]
fn native_metadata_maps_remain_strict_without_losing_legacy_defaults() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut state = native_state("11".repeat(32))?;
    state.settings.insert("saved".into(), json!({"on": false}));
    state.private_records.insert(
        "public-fixture".into(),
        SecretRecord::new(b"retained control".to_vec()),
    );
    let value = serde_json::to_value(&state)?;
    let encoded = serde_json::to_string(&value)?;
    for (field, key) in [("settings", "saved"), ("private_records", "public-fixture")] {
        let map = &value[field];
        let corrupt = replace_map(&encoded, field, map, &duplicate_entry(map, key, key)?)?;
        let path = directory.path().join(format!("{field}.json"));
        write_native(&path, &state, &corrupt)?;
        assert!(
            FileStorage::open(&path)?.load().is_err(),
            "accepted duplicate {field} identity"
        );
        assert_eq!(std::fs::read(&path)?, corrupt.as_bytes());
    }
    let mut legacy = serde_json::to_value(native_state("11".repeat(32))?)?;
    let fields = legacy.as_object_mut().ok_or("missing legacy object")?;
    fields.remove("mode");
    fields.remove("private_records");
    let restored: State = serde_json::from_value(legacy.clone())?;
    restored.validate()?;
    assert_eq!(restored.mode, StoreMode::Durable);
    assert!(restored.private_records.is_empty());
    legacy["future_field"] = json!(true);
    assert!(serde_json::from_value::<State>(legacy).is_err());
    Ok(())
}

#[derive(Clone)]
struct ReopenBackend(Arc<Mutex<State>>);

impl Storage for ReopenBackend {
    fn load(&self) -> kiaro_secret_chat::error::Result<Option<State>> {
        Ok(Some(
            self.0
                .lock()
                .map_err(|_| kiaro_secret_chat::error::Error::Storage)?
                .clone(),
        ))
    }

    fn commit(&mut self, state: &State) -> kiaro_secret_chat::error::Result<()> {
        state.validate()?;
        *self
            .0
            .lock()
            .map_err(|_| kiaro_secret_chat::error::Error::Storage)? = state.clone();
        Ok(())
    }
}

#[test]
fn bounded_control_and_age_maps_refuse_duplicates_on_open_and_resume() -> TestResult {
    let authorization = "11".repeat(32);
    let limits = CustodyLimits::new(32, 1024 * 1024, 3600.0)?;
    let mut store = CustodyStorage::new(
        MemoryStorage::default(),
        authorization.clone(),
        limits.clone(),
    )?;
    let mut state = store.load()?.ok_or("missing bounded state")?;
    state.chats = native_state(authorization.clone())?.chats;
    store.commit(&state)?;
    let state = store.load()?.ok_or("missing bounded state")?;
    let controls: Value = serde_json::from_slice(state.private_records["custody/v1"].as_bytes())?;
    let ages = &controls["7"]["ages"];
    let age = ages
        .as_object()
        .and_then(|ages| ages.keys().next())
        .ok_or("missing retained age")?;
    let corruptions = [
        duplicate_entry(&controls, "7", "7")?,
        duplicate_entry(&controls, "7", "07")?,
        replace_map(
            &serde_json::to_string(&controls)?,
            "ages",
            ages,
            &duplicate_entry(ages, age, age)?,
        )?,
    ];
    for corrupt in corruptions {
        let shared = ReopenBackend(Arc::new(Mutex::new(state.clone())));
        let mut resumed =
            CustodyStorage::new(shared.clone(), authorization.clone(), limits.clone())?;
        let before = serde_json::to_value(resumed.load()?.ok_or("missing bounded state")?)?;
        let mut invalid = state.clone();
        invalid
            .private_records
            .insert("custody/v1".into(), SecretRecord::new(corrupt.into_bytes()));
        *shared.0.lock().map_err(|_| "fixture lock poisoned")? = invalid.clone();
        assert!(resumed.resume_storage().is_err());
        assert!(serde_json::to_value(resumed.load()?.ok_or("missing retained state")?)? == before);
        let mut backend = MemoryStorage::default();
        backend.commit(&invalid)?;
        assert!(CustodyStorage::new(backend, authorization.clone(), limits.clone()).is_err());
    }
    Ok(())
}

#[test]
fn autosave_pending_duplicates_are_refused_before_settlement_rewrites_the_journal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let (archive, _) = AutoSave::enabled(directory.path())?;
    let key = FileKey::new(&[0x61; 32], &[0x72; 32])?;
    let reference = MediaReference::new(
        7,
        &grammers_tl_types::types::EncryptedFile {
            id: 77,
            access_hash: 88,
            size: 16,
            dc_id: 2,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: 1,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )?;
    archive.owe(7, 101, &reference, "retained.bin")?;
    assert_eq!(archive.pending(7)?.len(), 1);
    assert_eq!(archive.owed_downloads()?.len(), 1);
    let path = directory.path().join("7/pending.json");
    let unique = std::fs::read(&path)?;
    let map: Value = serde_json::from_slice(&unique)?;
    for alias in ["101", "0101"] {
        let corrupt = duplicate_entry(&map, "101", alias)?;
        std::fs::write(&path, &corrupt)?;
        assert!(archive.pending(7).is_err());
        assert!(archive.settle(7, 101).is_err());
        assert_eq!(std::fs::read(&path)?, corrupt.as_bytes());
    }
    std::fs::write(&path, unique)?;
    archive.settle(7, 101)?;
    assert!(archive.pending(7)?.is_empty());
    Ok(())
}
