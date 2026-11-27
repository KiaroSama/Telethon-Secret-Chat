#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, ManagerLimits, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, encrypt_frame, wrap_frame};
use kiaro_secret_chat::runtime_modes::PersistentSecretChatManager;
use kiaro_secret_chat::storage_backend::{
    FileStorage, PersistedChatRecord, PersistedStoreSnapshot, SecretChatStore,
};
use serde_json::json;

const SCALES: [u32; 4] = [1, 16, 64, 128];
const STORE_KEY: [u8; 32] = [0x32; 32];
const CHAT_KEY: [u8; 256] = [0x42; 256];
const OWNER: &str = "checkpoint-benchmark";
const CHAT_ID: i32 = 7;
const ENVELOPE_OVERHEAD: u64 = 8 + 12 + 16;
const RUNTIME_CEILING: Duration = Duration::from_secs(120);

struct OfflineTransport;

impl SecretTransport for OfflineTransport {
    fn auth_identity(&self) -> &str {
        OWNER
    }

    fn send_encrypted(
        &self,
        _: i64,
        _: i64,
        _: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { panic!("the receive workload must not dispatch a data RPC") })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> BoxFuture<'_, Result<(), SecretChatError>> {
        Box::pin(async { panic!("the receive workload must not discard the chat") })
    }
}

struct Sample {
    elapsed: Duration,
    serialized_bytes: u64,
    receipt_bytes: usize,
}

struct MeasuredStore<'a> {
    store: FileStorage,
    path: &'a Path,
    samples: &'a Mutex<Vec<Sample>>,
}

impl SecretChatStore for MeasuredStore<'_> {
    fn load_all(&self) -> Result<PersistedStoreSnapshot, SecretChatError> {
        self.store.load_all()
    }

    fn save(&self, record: &PersistedChatRecord) -> Result<(), SecretChatError> {
        self.store.save(record)
    }

    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<(), SecretChatError> {
        let start = Instant::now();
        self.store.save_checkpoint(expected, record)?;
        let elapsed = start.elapsed();

        let disk_bytes = fs::metadata(self.path).expect("checkpoint metadata").len();
        let snapshot = FileStorage::open(self.path, STORE_KEY, OWNER)?.load_all()?;
        let reopened = serde_json::to_value(&snapshot.chats[&CHAT_ID]).expect("reopened record");
        assert_eq!(
            reopened,
            serde_json::to_value(record).expect("written record")
        );
        let receipt_bytes = reopened.get("pending_deliveries").map_or(0, |receipts| {
            serde_json::to_vec(receipts).expect("receipt JSON").len()
        });
        let serialized_bytes = disk_bytes - ENVELOPE_OVERHEAD;
        assert_eq!(
            serialized_bytes,
            serde_json::to_vec(&snapshot).expect("snapshot JSON").len() as u64
        );
        self.samples
            .lock()
            .expect("benchmark samples")
            .push(Sample {
                elapsed,
                serialized_bytes,
                receipt_bytes,
            });
        Ok(())
    }

    fn delete(&self, chat_id: i32) -> Result<(), SecretChatError> {
        self.store.delete(chat_id)
    }

    fn mark_transient_suspended(&self, chat_id: i32) -> Result<(), SecretChatError> {
        self.store.mark_transient_suspended(chat_id)
    }
}

#[tokio::test]
async fn measures_encrypted_checkpoint_and_receipt_serialization()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let text = "x".repeat(1024);
    let frames = (1..=SCALES[3])
        .map(|order| {
            let order = i64::from(order);
            let body = wrap_frame(
                &json!({"random_id": order, "ttl": 0, "message": &text}),
                46,
                0,
                order * 2 - 1,
            )?;
            encrypt_frame(&CHAT_KEY, &body, 0)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let benchmark_started = Instant::now();
    for messages in SCALES {
        let path = dir.path().join(format!("checkpoint workload {messages}"));
        let samples = Mutex::new(Vec::new());
        let store = MeasuredStore {
            store: FileStorage::open(&path, STORE_KEY, OWNER)?,
            path: &path,
            samples: &samples,
        };
        let manager =
            PersistentSecretChatManager::open(OfflineTransport, store, ManagerLimits::default())
                .await?;
        manager
            .register_and_persist(PersistedChatRecord::established(
                CHAT_ID,
                101,
                202,
                ChatRole::Acceptor,
                CHAT_KEY,
            )?)
            .await?;
        samples.lock().expect("benchmark samples").clear();

        let started = Instant::now();
        for (index, frame) in frames.iter().take(messages as usize).enumerate() {
            assert!(
                benchmark_started.elapsed() < RUNTIME_CEILING,
                "checkpoint benchmark stalled"
            );
            let accepted = manager.handle_incoming(CHAT_ID, frame).await?;
            assert_eq!(accepted.len(), 1);
            assert_eq!(accepted[0].random_id, index as i64 + 1);
            assert_eq!(accepted[0].text.as_deref(), Some(text.as_str()));
        }
        let elapsed = started.elapsed();
        drop(manager);
        let samples = samples.into_inner()?;
        assert_eq!(samples.len(), 2 * messages as usize);
        let (pairs, remainder) = samples.as_chunks::<2>();
        assert!(remainder.is_empty());
        for pair in pairs {
            assert!(pair[0].receipt_bytes > text.len());
            assert_eq!(pair[1].receipt_bytes, 0);
            assert!(pair[0].serialized_bytes > pair[1].serialized_bytes);
        }
        let final_record = FileStorage::open(&path, STORE_KEY, OWNER)?
            .get(CHAT_ID)?
            .expect("last checkpoint");
        assert_eq!(final_record.in_seq_no, i32::try_from(messages)?);
        assert_eq!(final_record.auth_key, CHAT_KEY);

        let checkpoint_time: Duration = samples.iter().map(|sample| sample.elapsed).sum();
        let checkpoint_bytes: u64 = samples.iter().map(|sample| sample.serialized_bytes).sum();
        let receipt_bytes: usize = samples.iter().map(|sample| sample.receipt_bytes).sum();
        let peak_checkpoint_bytes = samples.iter().map(|sample| sample.serialized_bytes).max();
        let mut latencies: Vec<_> = samples.iter().map(|sample| sample.elapsed).collect();
        latencies.sort_unstable();
        let p95 = latencies[(95 * latencies.len()).div_ceil(100) - 1];
        eprintln!(
            "messages={messages} checkpoints={} wall={elapsed:?} avg_message_wall={:?} \
             checkpoint_wall={checkpoint_time:?} avg_checkpoint_time_per_message={:?} \
             checkpoint_p95={p95:?} checkpoint_json_bytes={checkpoint_bytes} \
             avg_checkpoint_json_bytes={} peak_checkpoint_json_bytes={} \
             receipt_json_bytes={receipt_bytes}",
            samples.len(),
            elapsed / messages,
            checkpoint_time / messages,
            checkpoint_bytes / samples.len() as u64,
            peak_checkpoint_bytes.unwrap_or_default(),
        );
    }
    assert!(
        benchmark_started.elapsed() < RUNTIME_CEILING,
        "checkpoint benchmark exceeded its runtime ceiling"
    );
    Ok(())
}
