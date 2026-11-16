#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
#[path = "support_manager/transient.rs"]
mod transient;
use kiaro_secret_chat::{
    chat::Exchange,
    crypto::SharedKey,
    custody::{ProtectedFileStorage, TransientLimits, TransientStorage},
    dh::SecretExponent,
    manager::{Options, SecretChatManager},
    storage::Storage,
};
use std::sync::Arc;
use support::*;
use transient::*;
async fn reconstruct_phase(phase: u8) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.json");
    let (a, b, w, id) = pair(
        ProtectedFileStorage::open(&path).unwrap(),
        TransientLimits::default(),
    )
    .await;
    a.stop().await.unwrap();
    let mut store = ProtectedFileStorage::open(&path).unwrap();
    let mut state = store.load().unwrap().unwrap();
    let chat = state.chats.get_mut(&id).unwrap();
    chat.previous_key = Some(SharedKey::from_slice(&[5; 256]).unwrap());
    chat.exchange = match phase {
        0 => Some(Exchange::Requested {
            id: 42,
            secret: SecretExponent::generate().unwrap(),
        }),
        1 => Some(Exchange::Accepted {
            id: 42,
            key: SharedKey::from_slice(&[3; 256]).unwrap(),
        }),
        2 => {
            chat.adopt_key(SharedKey::from_slice(&[3; 256]).unwrap())
                .unwrap();
            chat.new_key_confirmed = false;
            None
        }
        _ => panic!("unknown phase"),
    };
    let before = serde_json::to_value(&chat).unwrap();
    store.commit(&state).unwrap();
    drop(store);
    let count = w.lock().unwrap().sent.len();
    let restored = SecretChatManager::new(
        Fake {
            user: 1000,
            world: Arc::clone(&w),
        },
        TransientStorage::new(
            ProtectedFileStorage::open(&path).unwrap(),
            "11".repeat(32),
            TransientLimits::default(),
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    restored.start().await.unwrap();
    assert!(restored.is_suspended(id).unwrap());
    assert_eq!(w.lock().unwrap().sent.len(), count);
    assert!(restored.retry_pending(id).await.is_err());
    restored.stop().await.unwrap();
    let after = serde_json::to_value(
        ProtectedFileStorage::open(&path)
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .chats
            .get(&id)
            .unwrap(),
    )
    .unwrap();
    for field in [
        "key",
        "previous_key",
        "exchange",
        "new_key_confirmed",
        "in_seq_no",
        "out_seq_no",
    ] {
        assert_eq!(before[field], after[field], "protected field {field}");
    }
    b.stop().await.unwrap();
}
#[tokio::test]
async fn requested_rekey_crash_retains_keys_and_refuses_replay() {
    reconstruct_phase(0).await;
}
#[tokio::test]
async fn accepted_rekey_crash_retains_keys_and_refuses_replay() {
    reconstruct_phase(1).await;
}
#[tokio::test]
async fn committed_rekey_crash_retains_keys_and_refuses_replay() {
    reconstruct_phase(2).await;
}
