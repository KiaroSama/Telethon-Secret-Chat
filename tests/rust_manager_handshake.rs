#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use grammers_tl_types as tl;
use kiaro_secret_chat::{
    chat::ChatState,
    dh::{DhConfig, SecretExponent},
    manager::Event,
};
use std::sync::{Arc, Mutex};
use support::*;
fn id(chat: &tl::enums::EncryptedChat) -> i32 {
    chat.id()
}
#[tokio::test]
async fn early_discard_before_request_returns_is_terminal() {
    let (a, _b, world) = managers();
    let manager = a.clone();
    world.lock().unwrap().request_hook = Some(Box::new(move |chat| {
        Box::pin(async move {
            manager
                .handle_encryption(
                    tl::types::EncryptedChatDiscarded {
                        history_deleted: false,
                        id: id(&chat),
                    }
                    .into(),
                )
                .await
                .unwrap();
        })
    }));
    let snapshot = a.create(peer(2000)).await.unwrap();
    assert_eq!(snapshot.state, ChatState::Closed);
    assert!(snapshot.key_fingerprint.is_none());
}
#[tokio::test]
async fn early_ready_before_request_returns_is_established() {
    lost_create_reply_recovers_original_request_and_dh_identity().await;
    let (a, _b, world) = managers();
    let manager = a.clone();
    let server = Arc::clone(&world);
    let expected = Arc::new(Mutex::new(None));
    let observed = Arc::clone(&expected);
    world.lock().unwrap().request_hook = Some(Box::new(move |chat| {
        Box::pin(async move {
            let (admin, participant, ga) = server.lock().unwrap().requests[&id(&chat)].clone();
            let dh = DhConfig::new(2, &prime()).unwrap();
            let secret = SecretExponent::generate().unwrap();
            let key = dh.shared_key(&ga, &secret).unwrap();
            *observed.lock().unwrap() = Some(key.fingerprint());
            manager
                .handle_encryption(
                    tl::types::EncryptedChat {
                        id: id(&chat),
                        access_hash: 88,
                        date: 1,
                        admin_id: admin,
                        participant_id: participant,
                        g_a_or_b: dh.public_value(&secret),
                        key_fingerprint: key.fingerprint(),
                    }
                    .into(),
                )
                .await
                .unwrap();
        })
    }));
    let chat = a.create(peer(2000)).await.unwrap();
    assert_eq!(chat.state, ChatState::Ready);
    assert_eq!(chat.key_fingerprint, *expected.lock().unwrap());
}
#[tokio::test]
async fn already_accepted_server_reply_converges_locally() {
    let (a, b, world) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    world.lock().unwrap().fail_accept =
        Some(kiaro_secret_chat::manager::RpcFailure::AlreadyAccepted);
    b.accept(id).await.unwrap();
    assert_eq!(b.status(id).unwrap().state, ChatState::Ready);
    assert!(b.status(id).unwrap().key_fingerprint.is_some());
}
#[tokio::test]
async fn declined_server_reply_closes_pending_record() {
    let (a, b, world) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    world.lock().unwrap().fail_accept = Some(kiaro_secret_chat::manager::RpcFailure::Declined);
    assert!(b.accept(id).await.is_err());
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    assert!(b.status(id).unwrap().key_fingerprint.is_none());
}
#[tokio::test]
async fn duplicate_request_never_replaces_ready_key_or_accepts_twice() {
    use kiaro_secret_chat::{
        manager::{Options, SecretChatManager},
        storage::MemoryStorage,
    };
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let a = SecretChatManager::new(
        aa,
        MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(
        bb.clone(),
        store.clone(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    assert_eq!(b.status(id).unwrap().state, ChatState::Waiting);
    b.stop().await.unwrap();
    let b = SecretChatManager::new(bb, store, "22".repeat(32), Options::default()).unwrap();
    b.start().await.unwrap();
    assert_eq!(b.status(id).unwrap().state, ChatState::Waiting);
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    let fingerprint = b.status(id).unwrap().key_fingerprint;
    let ga = world.lock().unwrap().requests[&id].2.clone();
    b.handle_encryption(
        tl::types::EncryptedChatRequested {
            folder_id: None,
            id,
            access_hash: 88,
            date: 1,
            admin_id: 1000,
            participant_id: 2000,
            g_a: ga,
        }
        .into(),
    )
    .await
    .unwrap();
    b.accept(id).await.unwrap();
    assert_eq!(b.status(id).unwrap().key_fingerprint, fingerprint);
    assert_eq!(world.lock().unwrap().accepts.len(), 1);
    assert_eq!(a.status(id).unwrap().key_fingerprint, fingerprint);
}
#[tokio::test]
async fn late_ready_cannot_revive_closed_tombstone() {
    let (a, _b, _world, id) = established().await;
    a.close(id).await.unwrap();
    a.handle_encryption(
        tl::types::EncryptedChat {
            id,
            access_hash: 88,
            date: 1,
            admin_id: 1000,
            participant_id: 2000,
            g_a_or_b: b"not processed".to_vec(),
            key_fingerprint: 0,
        }
        .into(),
    )
    .await
    .unwrap();
    assert_eq!(a.status(id).unwrap().state, ChatState::Closed);
    assert!(a.status(id).unwrap().key_fingerprint.is_none());
}
#[tokio::test]
async fn wrong_initial_fingerprint_closes_and_discards() {
    let (a, _b, world) = managers();
    let chat = a.create(peer(2000)).await.unwrap();
    let ga = world.lock().unwrap().requests[&chat.id].2.clone();
    assert!(
        a.handle_encryption(
            tl::types::EncryptedChat {
                id: chat.id,
                access_hash: 88,
                date: 1,
                admin_id: 1000,
                participant_id: 2000,
                g_a_or_b: ga,
                key_fingerprint: 12345
            }
            .into()
        )
        .await
        .is_err()
    );
    assert_eq!(a.status(chat.id).unwrap().state, ChatState::Closed);
    assert_eq!(world.lock().unwrap().discards, [(chat.id, false)]);
}
#[tokio::test]
async fn unknown_waiting_request_is_discarded_and_reported() {
    let (a, _b, world) = managers();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    a.on("DecryptFailed", "lost", move |event| {
        if let Event::DecryptFailed { chat_id, reason } = event {
            sink.lock().unwrap().push((chat_id, reason));
        }
        Ok(())
    })
    .unwrap();
    a.handle_encryption(
        tl::types::EncryptedChatWaiting {
            id: 4242,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
        }
        .into(),
    )
    .await
    .unwrap();
    assert_eq!(world.lock().unwrap().discards, [(4242, false)]);
    assert_eq!(*seen.lock().unwrap(), [(4242, "request lost; discarded")]);
}
#[tokio::test]
async fn unknown_ready_outside_create_is_discarded() {
    let (a, _b, world) = managers();
    a.handle_encryption(
        tl::types::EncryptedChat {
            id: 4343,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
            g_a_or_b: vec![3; 256],
            key_fingerprint: 1,
        }
        .into(),
    )
    .await
    .unwrap();
    assert_eq!(world.lock().unwrap().discards, [(4343, false)]);
}
#[tokio::test]
async fn own_waiting_update_is_left_alone() {
    let (a, _b, world) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    a.handle_encryption(
        tl::types::EncryptedChatWaiting {
            id,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
        }
        .into(),
    )
    .await
    .unwrap();
    assert!(world.lock().unwrap().discards.is_empty());
    assert_eq!(a.status(id).unwrap().state, ChatState::Requested);
}
async fn lost_create_reply_recovers_original_request_and_dh_identity() {
    let (a, b, world) = managers();
    world.lock().unwrap().fail_request = Some(kiaro_secret_chat::manager::RpcFailure::Transient);
    assert!(a.create(peer(2000)).await.is_err());
    assert!(a.list().unwrap().is_empty());
    let before = world.lock().unwrap().requests.clone();
    a.start().await.unwrap();
    let ids = world.lock().unwrap().request_ids.clone();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(world.lock().unwrap().requests, before);
    assert_eq!(a.list().unwrap().len(), 1);
    let id = a.list().unwrap()[0].id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(a.status(id).unwrap().state, ChatState::Ready);
    assert_eq!(
        a.status(id).unwrap().key_fingerprint,
        b.status(id).unwrap().key_fingerprint
    );
}
#[tokio::test]
async fn cancelled_create_does_not_leave_creation_quarantine_active() {
    let (a, _b, world) = managers();
    let gate = Arc::new(Gate::default());
    let wait = Arc::clone(&gate);
    world.lock().unwrap().request_hook = Some(Box::new(move |_| {
        Box::pin(async move {
            wait.entered.notify_one();
            wait.release.notified().await;
        })
    }));
    let manager = a.clone();
    let creation = tokio::spawn(async move { manager.create(peer(2000)).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    a.handle_encryption(
        tl::types::EncryptedChatWaiting {
            id: 4444,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
        }
        .into(),
    )
    .await
    .unwrap();
    a.handle_encryption(
        tl::types::EncryptedChat {
            id: 4444,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
            g_a_or_b: vec![3; 256],
            key_fingerprint: 1,
        }
        .into(),
    )
    .await
    .unwrap();
    assert!(world.lock().unwrap().discards.is_empty());
    assert!(a.status(4444).is_err());
    creation.abort();
    assert!(creation.await.unwrap_err().is_cancelled());
    a.handle_encryption(
        tl::types::EncryptedChatWaiting {
            id: 4444,
            access_hash: 1,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
        }
        .into(),
    )
    .await
    .unwrap();
    assert_eq!(world.lock().unwrap().discards, [(4444, false)]);
}
#[tokio::test]
async fn invalid_dh_configuration_is_refused_before_request_rpc() {
    let (a, _b, world) = managers();
    let mut invalid = vec![0; 256];
    invalid[0] = 128;
    invalid[255] = 1;
    world.lock().unwrap().dh_prime_override = Some(invalid);
    assert!(a.create(peer(2000)).await.is_err());
    assert!(a.list().unwrap().is_empty());
    assert!(world.lock().unwrap().request_ids.is_empty());
}
#[tokio::test]
async fn unsafe_incoming_request_is_discarded_before_installation() {
    let (a, _b, world) = managers();
    let mut ga = vec![0; 256];
    ga[255] = 1;
    assert!(
        a.handle_encryption(
            tl::types::EncryptedChatRequested {
                folder_id: None,
                id: 7,
                access_hash: 88,
                date: 0,
                admin_id: 2000,
                participant_id: 1000,
                g_a: ga
            }
            .into()
        )
        .await
        .is_err()
    );
    assert!(a.list().unwrap().is_empty());
    assert_eq!(world.lock().unwrap().discards, [(7, false)]);
}
#[tokio::test]
async fn pending_request_accepted_elsewhere_closes_locally_only() {
    let (a, b, world) = managers();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.handle_encryption(
        tl::types::EncryptedChat {
            id,
            access_hash: 88,
            date: 0,
            admin_id: 1000,
            participant_id: 2000,
            g_a_or_b: b"not our exponent".to_vec(),
            key_fingerprint: 0,
        }
        .into(),
    )
    .await
    .unwrap();
    assert_eq!(b.status(id).unwrap().state, ChatState::Closed);
    assert!(world.lock().unwrap().discards.is_empty());
}
