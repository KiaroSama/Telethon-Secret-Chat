//! This suite owns a synthetic corpus with deterministic fixture keys and no account material.
//! `GRAMMERS_PROP_SEED` selects one decimal or 0x-prefixed seed. The default corpus
//! runs at most 35 transitions per seed, with a CI runtime target below two seconds.

#![forbid(unsafe_code)]

use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
};

use kiaro_secret_chat::{
    errors::SecretChatError,
    ingress::{
        DurableIngressController, EncryptedIngressStore, IngressLimits, IngressOrigin,
        IngressUpdate,
    },
    manager::{BoxFuture, ManagerLimits, SecretTransport},
    protocol::{AcceptedMessage, ChatRole},
    runtime_modes::{PersistentSecretChatManager, StrongSecretChatManager},
    storage_backend::{
        MemoryStorage, PersistedChatRecord, PersistedStoreSnapshot, SecretChatStore,
    },
};

const CHAT: i32 = 7;
const KEY: [u8; 256] = [0x6c; 256];
const ALICE: &str = "property-alice";
const BOB: &str = "property-bob";
const SEEDS: &[u64] = &[
    0x5eed_0001,
    0x5eed_0002,
    0x5eed_0003,
    0x5eed_0004,
    0x5eed_0005,
    0x5eed_0006,
    0x5eed_0007,
    0x5eed_0008,
    0x5eed_0009,
    0x5eed_000a,
    0x5eed_000b,
    0x5eed_000c,
    0x5eed_000d,
    0x5eed_000e,
    0x5eed_000f,
    0x5eed_0010,
    0x5eed_0011,
    0x5eed_0012,
    0x5eed_0013,
    0x5eed_0014,
    0x5eed_0015,
    0x5eed_0016,
    0x5eed_0017,
    0x5eed_0018,
    0x5eed_0019,
    0x5eed_001a,
    0x5eed_001b,
    0x5eed_001c,
    0x5eed_001d,
    0x5eed_001e,
    0x5eed_001f,
    0x5eed_0020,
    0x5eed_0021,
    0x5eed_0022,
    0x5eed_0023,
    0x5eed_0024,
    0x5eed_0025,
    0x5eed_0026,
    0x5eed_0027,
    0x5eed_0028,
    0x5eed_0029,
    0x5eed_002a,
];

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let z = (self.0 ^ (self.0 >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[derive(Default)]
struct Wire {
    fail_text: bool,
    text: VecDeque<Vec<u8>>,
    service: VecDeque<Vec<u8>>,
}

#[derive(Clone)]
struct Transport(&'static str, Arc<Mutex<Wire>>);

impl Transport {
    fn new(owner: &'static str) -> Self {
        Self(owner, Arc::default())
    }

    fn text(&self) -> Vec<u8> {
        self.1.lock().unwrap().text.pop_front().unwrap()
    }

    fn service(&self) -> Vec<u8> {
        self.1.lock().unwrap().service.pop_front().unwrap()
    }
}

impl SecretTransport for Transport {
    fn auth_identity(&self) -> &str {
        self.0
    }

    fn send_encrypted<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            let mut wire = self.1.lock().unwrap();
            if std::mem::take(&mut wire.fail_text) {
                return Err(SecretChatError::ParameterRejected {
                    chat_id: Some(i64::from(CHAT)),
                    reason: "synthetic transport failure".into(),
                });
            }
            wire.text.push_back(frame);
            Ok(())
        })
    }

    fn send_encrypted_service<'a>(
        &'a self,
        _: i64,
        _: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async move {
            self.1.lock().unwrap().service.push_back(frame);
            Ok(())
        })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct Store(Arc<MemoryStorage>);

impl Store {
    fn new(owner: &str) -> Self {
        Self(Arc::new(MemoryStorage::new(owner).unwrap()))
    }
}

impl SecretChatStore for Store {
    fn load_all(&self) -> Result<PersistedStoreSnapshot, SecretChatError> {
        self.0.load_all()
    }
    fn save(&self, record: &PersistedChatRecord) -> Result<(), SecretChatError> {
        self.0.save(record)
    }
    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<(), SecretChatError> {
        self.0.save_checkpoint(expected, record)
    }
    fn delete(&self, id: i32) -> Result<(), SecretChatError> {
        self.0.delete(id)
    }
    fn mark_transient_suspended(&self, id: i32) -> Result<(), SecretChatError> {
        self.0.mark_transient_suspended(id)
    }
}

type Mode = PersistentSecretChatManager<Transport, Store>;

fn record(role: ChatRole) -> PersistedChatRecord {
    PersistedChatRecord::established(CHAT, 8, 9, role, KEY).unwrap()
}

async fn open(transport: &Transport, store: &Store) -> Mode {
    Mode::open(transport.clone(), store.clone(), ManagerLimits::default())
        .await
        .unwrap()
}

async fn counters(mode: &Mode) -> (i64, i64) {
    let record = mode.snapshot_chat(CHAT).await.unwrap();
    (i64::from(record.in_seq_no), i64::from(record.out_seq_no))
}

async fn restart(mode: Mode, transport: &Transport, store: &Store) -> Mode {
    let before = counters(&mode).await;
    drop(mode);
    let mode = open(transport, store).await;
    assert_eq!(before, counters(&mode).await);
    mode
}

fn texts(messages: Vec<AcceptedMessage>) -> Vec<String> {
    messages
        .into_iter()
        .filter_map(|message| message.text)
        .collect()
}

async fn trace(seed: u64) {
    let mut rng = SplitMix64(seed);
    let steps = 20 + rng.next() % 16;
    let alice_wire = Transport::new(ALICE);
    let bob_wire = Transport::new(BOB);
    let alice_store = Store::new(ALICE);
    let bob_store = Store::new(BOB);
    let mut alice = open(&alice_wire, &alice_store).await;
    let mut bob = open(&bob_wire, &bob_store).await;
    alice
        .register_and_persist(record(ChatRole::Initiator))
        .await
        .unwrap();
    bob.register_and_persist(record(ChatRole::Acceptor))
        .await
        .unwrap();
    let mut queued = VecDeque::new();
    let mut delivered = Vec::new();
    let mut previous = [counters(&alice).await, counters(&bob).await];

    for step in 0..steps {
        let choice = if step < 4 {
            [0, 0, 2, 3][step as usize]
        } else if step + 1 == steps {
            3
        } else {
            rng.next() % 7
        };
        match choice {
            0 | 1 => {
                let text = format!("{seed:#x}/{step}");
                if choice == 1 {
                    alice_wire.1.lock().unwrap().fail_text = true;
                    assert!(alice.send_text(CHAT, &text).await.is_err());
                    alice = restart(alice, &alice_wire, &alice_store).await;
                    assert_eq!(alice.retry_pending(CHAT).await.unwrap(), 1);
                } else {
                    assert_eq!(
                        alice.send_text(CHAT, &text).await.unwrap(),
                        alice_wire.1.lock().unwrap().text[0]
                    );
                }
                queued.push_back((alice_wire.text(), text));
            }
            2 if queued.len() >= 2 => {
                let late = &queued[1].0;
                for _ in 0..2 {
                    assert!(texts(bob.handle_incoming(CHAT, late).await.unwrap()).is_empty());
                }
                bob = restart(bob, &bob_wire, &bob_store).await;
            }
            3 => {
                let mut actual = Vec::new();
                let mut expected = Vec::new();
                while let Some((frame, text)) = queued.pop_front() {
                    actual.extend(texts(bob.handle_incoming(CHAT, &frame).await.unwrap()));
                    assert!(texts(bob.handle_incoming(CHAT, &frame).await.unwrap()).is_empty());
                    expected.push(text);
                }
                assert_eq!(actual, expected, "seed {seed:#x}, step {step}");
                delivered.extend(actual);
            }
            4 => alice = restart(alice, &alice_wire, &alice_store).await,
            5 => bob = restart(bob, &bob_wire, &bob_store).await,
            6 if queued.is_empty() => {
                let ttl = (rng.next() % 60) as i32;
                let frame = alice.set_ttl(CHAT, ttl).await.unwrap();
                assert!(texts(bob.handle_incoming(CHAT, &frame).await.unwrap()).is_empty());
                assert_eq!(bob.snapshot_chat(CHAT).await.unwrap().ttl, ttl);
            }
            _ => {}
        }
        let current = [counters(&alice).await, counters(&bob).await];
        for (before, after) in previous.into_iter().zip(current) {
            assert!(
                before.0 <= after.0 && before.1 <= after.1,
                "seed {seed:#x}, step {step}"
            );
        }
        assert_eq!(
            current[0].1 - current[1].0,
            queued.len() as i64,
            "seed {seed:#x}, step {step}"
        );
        previous = current;
    }
    assert!(queued.is_empty());
    assert!(delivered.len() >= 2);
}

#[tokio::test]
async fn seeded_receive_retry_and_restart_traces() {
    match std::env::var("GRAMMERS_PROP_SEED") {
        Ok(seed) => {
            let seed = seed
                .strip_prefix("0x")
                .map_or_else(|| seed.parse(), |hex| u64::from_str_radix(hex, 16))
                .unwrap();
            trace(seed).await;
        }
        Err(std::env::VarError::NotPresent) => {
            for &seed in SEEDS {
                trace(seed).await;
            }
        }
        Err(error) => panic!("invalid GRAMMERS_PROP_SEED: {error}"),
    }
}

type Strong = StrongSecretChatManager<Transport, Store>;

async fn open_strong(path: &Path, wire: &Transport, store: &Store) -> Strong {
    let controller = DurableIngressController::restore(
        BOB,
        EncryptedIngressStore::open(path, [0x61; 32]).unwrap(),
        IngressLimits::default(),
    )
    .unwrap();
    Strong::new(open(wire, store).await, controller).unwrap()
}

#[tokio::test]
async fn strong_delivery_replays_after_restart_before_ack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingress.store");
    let wire = Transport::new(BOB);
    let store = Store::new(BOB);
    let mut bob = open_strong(&path, &wire, &store).await;
    bob.register_and_persist(record(ChatRole::Acceptor))
        .await
        .unwrap();
    let alice = open(&Transport::new(ALICE), &Store::new(ALICE)).await;
    alice
        .register_and_persist(record(ChatRole::Initiator))
        .await
        .unwrap();
    for order in 1..=3 {
        let text = format!("ack-{order}");
        bob.ingest_before_ack(IngressUpdate {
            stream_id: Strong::stream_id(CHAT),
            order,
            provenance: IngressOrigin::VerifiedTransport,
            associated_chat_id: Some(i64::from(CHAT)),
            payload: alice.send_text(CHAT, &text).await.unwrap(),
        })
        .await
        .unwrap();
        assert_eq!(bob.applied_cursor(CHAT).await.unwrap(), order - 1);
        let accepted = bob.apply_delivery(CHAT, order).await.unwrap();
        assert_eq!(texts(accepted.clone()), [text]);
        drop(bob);
        bob = open_strong(&path, &wire, &store).await;
        let replay = bob.replay_pending().await.unwrap();
        assert!(replay.chat_errors.is_empty());
        assert_eq!(replay.applied, [(CHAT, order, accepted)]);
        assert_eq!(bob.applied_cursor(CHAT).await.unwrap(), order - 1);
        assert!(bob.acknowledge_delivery(CHAT, order).await.unwrap());
        assert_eq!(bob.applied_cursor(CHAT).await.unwrap(), order);
        assert!(bob.replay_pending().await.unwrap().applied.is_empty());
    }
}

#[tokio::test]
async fn rekey_completes_after_request_checkpoint_is_reopened() {
    let alice_wire = Transport::new(ALICE);
    let bob_wire = Transport::new(BOB);
    let alice_store = Store::new(ALICE);
    let bob_store = Store::new(BOB);
    let alice = open(&alice_wire, &alice_store).await;
    let bob = open(&bob_wire, &bob_store).await;
    // RFC 3526 group 14 uses public parameters.
    let prime = hex::decode(concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
        "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
        "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
        "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    ))
    .unwrap();
    let mut cancelled = kiaro_secret_chat::protocol::SecretChat::new(
        i64::from(CHAT),
        8,
        9,
        ALICE,
        ChatRole::Initiator,
    );
    let initial = cancelled.adopt_key(&KEY).unwrap();
    cancelled.configure_dh(2, &prime).unwrap();
    cancelled.start_rekey().unwrap().unwrap();
    let kiaro_secret_chat::protocol::KeyLifecycleState::Rekeying { exchange_id, .. } =
        cancelled.key_lifecycle()
    else {
        panic!("start_rekey did not stage an exchange");
    };
    let abort = serde_json::json!({"id": 0xdd05ec6b_u32, "fields": {"exchange_id": exchange_id}});
    for _ in 0..2 {
        assert!(cancelled.handle_rekey_action(&abort).unwrap().is_empty());
        assert_eq!(
            cancelled.key_lifecycle(),
            kiaro_secret_chat::protocol::KeyLifecycleState::Active {
                fingerprint: initial
            }
        );
    }

    for (peer, role) in [(&alice, ChatRole::Initiator), (&bob, ChatRole::Acceptor)] {
        peer.register_and_persist(record(role)).await.unwrap();
        peer.configure_chat_dh(CHAT, 2, &prime).await.unwrap();
    }
    assert!(alice.start_rekey(CHAT).await.unwrap());
    let alice = restart(alice, &alice_wire, &alice_store).await;
    bob.handle_incoming(CHAT, &alice_wire.service())
        .await
        .unwrap();
    let bob = restart(bob, &bob_wire, &bob_store).await;
    alice
        .handle_incoming(CHAT, &bob_wire.service())
        .await
        .unwrap();
    bob.handle_incoming(CHAT, &alice_wire.service())
        .await
        .unwrap();
    alice
        .handle_incoming(CHAT, &bob_wire.service())
        .await
        .unwrap();
    let frame = alice.send_text(CHAT, "after rekey").await.unwrap();
    assert_eq!(
        texts(bob.handle_incoming(CHAT, &frame).await.unwrap()),
        ["after rekey"]
    );
}
