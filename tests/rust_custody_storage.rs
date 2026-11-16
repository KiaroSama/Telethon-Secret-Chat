//! Protected storage refuses payload channels; bounded controls never reset age.
#![allow(clippy::unwrap_used)]
#[path = "support_protocol/custody.rs"]
mod custody_regressions;
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, Envelope, Outgoing},
    crypto::SharedKey,
    custody::{
        CustodyLimits, CustodyStorage, ProtectedFileStorage, TransientLimits, TransientStorage,
    },
    error::{Error, Result},
    storage::{MemoryStorage, SecretRecord, State, Storage, StoreMode},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[derive(Clone, Default)]
struct Tracking {
    state: Arc<Mutex<Option<State>>>,
    failed: Arc<AtomicBool>,
    commits: Arc<AtomicUsize>,
}
impl Storage for Tracking {
    fn load(&self) -> Result<Option<State>> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::Storage);
        }
        state.validate()?;
        self.commits.fetch_add(1, Ordering::AcqRel);
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(())
    }
}
fn state() -> State {
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.mode = StoreMode::Protected;
    state
}
fn chat() -> Chat {
    let mut chat = Chat::new(7, 8, 9, true, ChatState::Waiting);
    chat.key = Some(SharedKey::from_slice(&[7; 256]).unwrap());
    chat.out_seq_no = 1;
    chat
}
fn outgoing() -> Outgoing {
    Outgoing {
        seq_no: 1,
        random_id: 55,
        frame: vec![7; 40],
        body: b"public ordinary fixture".to_vec(),
        failures: 0,
        service: false,
        file: None,
        pending: true,
    }
}
fn incoming() -> Envelope {
    Envelope {
        body: kiaro_secret_chat::framing::wrap(
            kiaro_secret_chat::types::DecryptedMessage {
                no_webpage: false,
                silent: false,
                random_id: 9,
                ttl: 0,
                message: "public fixture".into(),
                media: None,
                entities: None,
                via_bot_name: None,
                reply_to_random_id: None,
                grouped_id: None,
            }
            .into(),
            144,
            1,
            2,
        )
        .unwrap()
        .to_bytes(),
        file: None,
        date: 1,
    }
}
fn protected() -> (tempfile::TempDir, ProtectedFileStorage) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProtectedFileStorage::open(dir.path().join("protected.json")).unwrap();
    store.commit(&state()).unwrap();
    (dir, store)
}
#[test]
fn protected_refuses_ordinary_state_before_any_commit() {
    custody_regressions::protected_store_refuses_existing_normal_even_empty();
    custody_regressions::transient_snapshot_never_contains_message_or_service_frames();

    let (dir, mut store) = protected();
    let before = std::fs::read(dir.path().join("protected.json")).unwrap();
    let mut ordinary = state();
    ordinary.mode = StoreMode::Durable;
    ordinary.chats.insert(7, chat());
    assert!(store.commit(&ordinary).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("protected.json")).unwrap(),
        before
    );
}
fn content_channel(operation: u8) {
    let (dir, mut store) = protected();
    let mut base = state();
    base.chats.insert(7, chat());
    store.commit(&base).unwrap();
    let before = std::fs::read(dir.path().join("protected.json")).unwrap();
    let mut bad = base.clone();
    match operation {
        0 => {
            bad.chats.get_mut(&7).unwrap().gaps.insert(1, incoming());
        }
        1 => {
            bad.chats.get_mut(&7).unwrap().outbox.insert(1, outgoing());
        }
        2 => {
            bad.settings
                .insert("ordinary".into(), "public content".into());
        }
        3 => {
            bad.chats.remove(&7);
        }
        _ => unreachable!(),
    };
    assert!(store.commit(&bad).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("protected.json")).unwrap(),
        before
    );
}
#[test]
fn protected_refuses_gap_content() {
    content_channel(0);
}
#[test]
fn protected_refuses_outgoing_content() {
    content_channel(1);
}
#[test]
fn protected_refuses_setting_content() {
    content_channel(2);
}
#[test]
fn protected_refuses_key_deletion() {
    content_channel(3);
}
#[test]
fn protected_refuses_existing_ordinary_empty_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ordinary.json");
    let mut ordinary = kiaro_secret_chat::storage::FileStorage::open(&path).unwrap();
    ordinary
        .commit(&State::empty("11".repeat(32)).unwrap())
        .unwrap();
    drop(ordinary);
    let before = std::fs::read(&path).unwrap();
    assert!(ProtectedFileStorage::open(&path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[test]
fn failed_protected_commit_preserves_ram_and_keys() {
    custody_regressions::transient_stop_and_restart_preserve_keys_and_refuse_continuation();

    let backend = Tracking::default();
    let mut ram =
        TransientStorage::new(backend.clone(), "11".repeat(32), TransientLimits::default())
            .unwrap();
    let mut base = state();
    base.chats.insert(7, chat());
    ram.commit(&base).unwrap();
    let before = serde_json::to_vec(backend.state.lock().unwrap().as_ref().unwrap()).unwrap();
    backend.failed.store(true, Ordering::Release);
    let mut next = base.clone();
    next.chats.get_mut(&7).unwrap().outbox.insert(1, outgoing());
    next.chats.get_mut(&7).unwrap().out_seq_no = 2;
    assert!(ram.commit(&next).is_err());
    let held = ram.committed_view().unwrap();
    assert_eq!(held.chats[&7].out_seq_no, 1);
    assert!(held.chats[&7].outbox.is_empty());
    assert_eq!(
        serde_json::to_vec(backend.state.lock().unwrap().as_ref().unwrap()).unwrap(),
        before
    );
}
fn extra_field(name: &str) {
    let (dir, mut store) = protected();
    let mut base = state();
    base.chats.insert(7, chat());
    store.commit(&base).unwrap();
    let before = std::fs::read(dir.path().join("protected.json")).unwrap();
    let mut value = serde_json::to_value(&base).unwrap();
    value["chats"]["7"]
        .as_object_mut()
        .unwrap()
        .insert(name.into(), "public extra content".into());
    assert!(serde_json::from_value::<State>(value).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("protected.json")).unwrap(),
        before
    );
}
#[test]
fn protected_schema_rejects_extra_deliveries() {
    extra_field("pending_deliveries");
}
#[test]
fn protected_schema_rejects_unknown_field() {
    custody_regressions::transient_private_slot_requires_exact_handshake_shape();
    custody_regressions::transient_rejects_payload_settings();

    extra_field("unexpected");
}
#[test]
fn ram_record_limit_fails_before_protected_commit() {
    custody_regressions::transient_capacity_refuses_without_silent_eviction_and_continuation();

    let backend = Tracking::default();
    let limits = TransientLimits {
        records: 1,
        ..Default::default()
    };
    let mut ram = TransientStorage::new(backend.clone(), "11".repeat(32), limits).unwrap();
    let mut base = state();
    base.chats.insert(7, chat());
    base.chats.get_mut(&7).unwrap().outbox.insert(1, outgoing());
    ram.commit(&base).unwrap();
    let before = backend.commits.load(Ordering::Acquire);
    let mut next = base;
    next.chats.get_mut(&7).unwrap().gaps.insert(1, incoming());
    next.validate().unwrap(); // The policy, rather than malformed sequence data, must refuse.
    assert!(matches!(ram.commit(&next), Err(Error::Capacity)));
    assert_eq!(backend.commits.load(Ordering::Acquire), before);
    let held = ram.committed_view().unwrap();
    assert_eq!(held.chats[&7].outbox.len(), 1);
    assert!(held.chats[&7].gaps.is_empty());
}
#[test]
fn global_byte_limit_refuses_without_payload_commit() {
    custody_regressions::transient_external_payload_reservations_share_one_global_budget();

    let backend = Tracking::default();
    let limits = TransientLimits {
        bytes: 64 * 1024,
        frame_bytes: 16 * 1024,
        attempts: 1,
        ..Default::default()
    };
    let mut ram = TransientStorage::new(backend.clone(), "11".repeat(32), limits).unwrap();
    let mut base = state();
    base.chats.insert(7, chat());
    ram.commit(&base).unwrap();
    let before = backend.commits.load(Ordering::Acquire);
    let mut row = outgoing();
    row.body = vec![88; 20000];
    base.chats.get_mut(&7).unwrap().outbox.insert(1, row);
    assert!(ram.commit(&base).is_err());
    assert_eq!(backend.commits.load(Ordering::Acquire), before);
    assert!(ram.committed_view().unwrap().chats[&7].outbox.is_empty());
}
#[test]
fn attempt_limit_refuses_and_suspends_without_persisting_payload() {
    let backend = Tracking::default();
    let limits = TransientLimits {
        attempts: 1,
        ..Default::default()
    };
    let mut ram = TransientStorage::new(backend.clone(), "11".repeat(32), limits).unwrap();
    let mut base = state();
    base.chats.insert(7, chat());
    base.chats.get_mut(&7).unwrap().outbox.insert(1, outgoing());
    ram.commit(&base).unwrap();
    ram.note_attempt(7, 1).unwrap();
    assert!(ram.note_attempt(7, 1).is_err());
    let held = ram.committed_view().unwrap();
    assert_eq!(held.chats[&7].state, ChatState::Suspended);
    let protected = backend.state.lock().unwrap();
    let chat = &protected.as_ref().unwrap().chats[&7];
    assert!(chat.key.is_some() && chat.outbox.is_empty());
    assert_eq!(chat.out_seq_no, 1);
}
fn bad_control(value: serde_json::Value) {
    let mut backend = MemoryStorage::default();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.mode = StoreMode::Bounded;
    state.chats.insert(7, chat());
    state.private_records.insert(
        "custody/v1".into(),
        SecretRecord::new(serde_json::to_vec(&serde_json::json!({"7":value})).unwrap()),
    );
    backend.commit(&state).unwrap();
    assert!(
        CustodyStorage::new(
            backend,
            "11".repeat(32),
            CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap()
        )
        .is_err()
    );
}
#[test]
fn custody_rejects_lost_without_pause() {
    custody_regressions::bounded_capacity_pauses_and_keeps_original_wire();
    custody_regressions::bounded_close_is_preserving_and_nonresumable();
    custody_regressions::stopped_file_owner_hands_off_and_reloads_without_overwriting();

    bad_control(serde_json::json!({"pause":null,"lost":true,"ages":{}}));
}
#[test]
fn custody_rejects_invalid_age_structure() {
    bad_control(serde_json::json!({"pause":null,"lost":false,"ages":{"out:1":null}}));
}
#[test]
fn custody_limits_reject_zero_records() {
    custody_regressions::transient_limits_have_original_defaults();
    custody_regressions::transient_limits_refuse_nonfinite_times();
    custody_regressions::custody_limits_refuse_bounds();

    let mut value = CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap();
    value.records = 0;
    assert!(value.validate().is_err());
}
#[test]
fn custody_limits_reject_boolean_records() {
    let mut value =
        serde_json::to_value(CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap()).unwrap();
    value["records"] = true.into();
    assert!(serde_json::from_value::<CustodyLimits>(value).is_err());
}
#[test]
fn custody_limits_reject_infinite_bytes() {
    let mut value =
        serde_json::to_value(CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap()).unwrap();
    value["bytes"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<CustodyLimits>(value).is_err());
    assert!(serde_json::from_str::<CustodyLimits>("{\"bytes\":Infinity}").is_err());
}
#[test]
fn custody_limits_reject_nan_age() {
    let mut value = CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap();
    value.age_seconds = f64::NAN;
    assert!(value.validate().is_err());
}
#[test]
fn custody_limits_reject_excess_tasks() {
    let mut value = CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap();
    value.tasks = 65;
    assert!(value.validate().is_err());
}
#[test]
fn custody_rejects_orphan_identity_age() {
    custody_regressions::bounded_refuses_untracked_legacy_custody();

    bad_control(serde_json::json!({"pause":null,"lost":false,"ages":{"out:1":1}}));
}
#[test]
fn custody_rejects_future_age() {
    bad_control(serde_json::json!({"pause":null,"lost":false,"ages":{"out:1":u64::MAX}}));
}
#[test]
fn custody_rejects_unrecognized_control_fields() {
    bad_control(
        serde_json::json!({"pause":null,"lost":false,"ages":{},"relay":[{"seq_no":0,"body":"00"}]}),
    );
}
