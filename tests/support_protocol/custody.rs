#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    chat::{Chat, ChatState, DhRecord},
    crypto::SharedKey,
    custody::{
        CustodyLimits, CustodyStorage, ProtectedFileStorage, TransientLimits, TransientStorage,
    },
    protocol::Protocol,
    storage::{FileStorage, MemoryStorage, State, Storage, StoreMode},
};
fn auth() -> String {
    "11".repeat(32)
}
fn dh() -> DhRecord {
    let text = include_str!("../fixtures/dh-prime.hex");
    DhRecord {
        g: 2,
        prime: hex::decode(text.trim()).unwrap(),
    }
}
fn install<S: Storage>(p: &mut Protocol<S>) {
    p.start().unwrap();
    p.establish(
        Chat::new(1, 2, 3, true, ChatState::Waiting),
        SharedKey::from_slice(&[7; 256]).unwrap(),
        dh(),
        1,
    )
    .unwrap();
}
pub fn transient_limits_have_original_defaults() {
    let l = TransientLimits::default();
    assert_eq!(
        (
            l.chats,
            l.records,
            l.bytes,
            l.frame_bytes,
            l.tasks,
            l.attempts
        ),
        (64, 512, 8 * 1024 * 1024, 262144, 8, 3)
    );
    l.validate().unwrap();
}
pub fn transient_limits_refuse_nonfinite_times() {
    for time in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -1.0,
        3601.0,
    ] {
        let l = TransientLimits {
            retention_seconds: time,
            ..Default::default()
        };
        assert!(l.validate().is_err());
    }
}
pub fn custody_limits_refuse_bounds() {
    for n in [0, 4097] {
        assert!(CustodyLimits::new(n, 8192, 60.0).is_err());
    }
    for n in [0, 64 * 1024 * 1024 + 1] {
        assert!(CustodyLimits::new(10, n, 60.0).is_err());
    }
    for n in [0.0, f64::NAN, f64::INFINITY, 30.0 * 86400.0 + 1.0] {
        assert!(CustodyLimits::new(10, 8192, n).is_err());
    }
}
pub fn protected_store_refuses_existing_normal_even_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    {
        let mut normal = FileStorage::open(&path).unwrap();
        normal.commit(&State::empty(auth()).unwrap()).unwrap();
    }
    assert!(ProtectedFileStorage::open(&path).is_err());
}
pub fn transient_snapshot_never_contains_message_or_service_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let backend = TransientStorage::new(
        ProtectedFileStorage::open(&path).unwrap(),
        auth(),
        TransientLimits::default(),
    )
    .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    install(&mut p);
    let sent = p
        .send_message(1, "PAYLOAD_SENTINEL_✓".into(), None, None)
        .unwrap();
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(!disk.contains("PAYLOAD_SENTINEL"));
    assert!(!disk.contains(&hex::encode(&sent.outgoing.frame)));
    let state: State = serde_json::from_str(&disk).unwrap();
    assert_eq!(state.mode, StoreMode::Protected);
    assert!(state.chats[&1].outbox.is_empty());
    assert!(state.chats[&1].deliveries.is_empty());
    assert!(state.chats[&1].service_deliveries.is_empty());
    assert!(state.chats[&1].key.is_some());
}
pub fn transient_stop_and_restart_preserve_keys_and_refuse_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    {
        let backend = TransientStorage::new(
            ProtectedFileStorage::open(&path).unwrap(),
            auth(),
            TransientLimits::default(),
        )
        .unwrap();
        let mut p = Protocol::open(backend, auth()).unwrap();
        install(&mut p);
        p.stop().unwrap();
        p.start().unwrap();
        assert_eq!(p.snapshot(1).unwrap().state, ChatState::Suspended);
        assert!(p.send_message(1, "lost".into(), None, None).is_err());
    }
    let backend = TransientStorage::new(
        ProtectedFileStorage::open(&path).unwrap(),
        auth(),
        TransientLimits::default(),
    )
    .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    p.start().unwrap();
    assert!(p.chat_record(1).unwrap().key.is_some());
    assert!(p.resume(1).is_err());
}
pub fn transient_capacity_refuses_without_silent_eviction_and_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let limits = TransientLimits {
        records: 1,
        ..Default::default()
    };
    let backend =
        TransientStorage::new(ProtectedFileStorage::open(&path).unwrap(), auth(), limits).unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    install(&mut p);
    assert!(p.send_message(1, "exhaust".into(), None, None).is_err());
    assert_eq!(p.snapshot(1).unwrap().state, ChatState::Suspended);
    assert!(p.chat_record(1).unwrap().key.is_some());
}
pub fn transient_rejects_payload_settings() {
    let backend =
        TransientStorage::new(MemoryStorage::default(), auth(), TransientLimits::default())
            .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    p.start().unwrap();
    assert!(
        p.set_setting("payload".into(), Some("ordinary".into()))
            .is_err()
    );
}
pub fn transient_private_slot_requires_exact_handshake_shape() {
    let backend =
        TransientStorage::new(MemoryStorage::default(), auth(), TransientLimits::default())
            .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    p.start().unwrap();
    let value =
        kiaro_secret_chat::storage::SecretRecord::new(br#"{"payload":"ordinary"}"#.to_vec());
    assert!(
        p.set_private_record("pending-create/1".into(), Some(value))
            .is_err()
    );
}
pub fn bounded_refuses_untracked_legacy_custody() {
    let mut backend = MemoryStorage::default();
    let mut state = State::empty(auth()).unwrap();
    state.mode = StoreMode::Bounded;
    state
        .chats
        .insert(1, Chat::new(1, 2, 3, true, ChatState::Waiting));
    backend.commit(&state).unwrap();
    assert!(
        CustodyStorage::new(
            backend,
            auth(),
            CustodyLimits::new(10, 65536, 60.0).unwrap()
        )
        .is_err()
    );
}
pub fn bounded_capacity_pauses_and_keeps_original_wire() {
    let backend = CustodyStorage::new(
        MemoryStorage::default(),
        auth(),
        CustodyLimits::new(1, 65536, 3600.0).unwrap(),
    )
    .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    install(&mut p);
    let before = p.retry_pending(1).unwrap().remove(0).outgoing.frame;
    assert!(p.send_message(1, "exhaust".into(), None, None).is_err());
    let chat = p.chat_record(1).unwrap();
    assert!(chat.paused);
    assert_eq!(chat.outbox.values().next().unwrap().frame, before);
    assert!(chat.key.is_some());
}
pub fn bounded_close_is_preserving_and_nonresumable() {
    let backend = CustodyStorage::new(
        MemoryStorage::default(),
        auth(),
        CustodyLimits::new(10, 65536, 3600.0).unwrap(),
    )
    .unwrap();
    let mut p = Protocol::open(backend, auth()).unwrap();
    install(&mut p);
    let count = p.chat_record(1).unwrap().outbox.len();
    p.terminate_local(1).unwrap();
    let chat = p.chat_record(1).unwrap();
    assert!(chat.paused);
    assert!(chat.key.is_some());
    assert_eq!(chat.outbox.len(), count);
    assert!(p.resume(1).is_err());
}
pub fn stopped_file_owner_hands_off_and_reloads_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let mut first = Protocol::open(FileStorage::open(&path).unwrap(), auth()).unwrap();
    first.start().unwrap();
    first.set_setting("owner".into(), Some(1.into())).unwrap();
    first.stop().unwrap();
    {
        let mut second = Protocol::open(FileStorage::open(&path).unwrap(), auth()).unwrap();
        second.start().unwrap();
        second.set_setting("owner".into(), Some(2.into())).unwrap();
        second.stop().unwrap();
    }
    first.stop().unwrap();
    first.start().unwrap();
    assert_eq!(first.setting("owner"), Some(2.into()));
}

pub fn transient_external_payload_reservations_share_one_global_budget() {
    let limits = TransientLimits {
        bytes: 8192,
        frame_bytes: 4096,
        ..Default::default()
    };
    let mut backend = TransientStorage::new(MemoryStorage::default(), auth(), limits).unwrap();
    backend.reserve_external(None, 1, 7000).unwrap();
    let mut state = backend.load().unwrap().unwrap();
    state
        .chats
        .insert(1, Chat::new(1, 2, 3, true, ChatState::Waiting));
    assert!(backend.commit(&state).is_err());
    assert!(backend.reserve_external(None, 2, 2000).is_err());
    backend.release_external(1);
    backend.reserve_external(None, 2, 2000).unwrap();
    backend.commit(&state).unwrap();
    backend.release_external(2);
}
