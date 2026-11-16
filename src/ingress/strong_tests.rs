//! Burned synthetic protocol fixtures on the real pinned SDK receipt boundary.
//! These tests do not stand in for the separate successful native-client proof.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::{
    custody::IngressLimits,
    manager::{RpcFailure, StrongSecretChatManager},
};
use grammers_mtsender::DurableIngressHook;
use grammers_tl_types::{self as tl, Serializable};
use std::{sync::Arc, time::Duration};
#[path = "../../tests/support_manager/compile_api.rs"]
mod compile_boundary;
use crate::test_support::support as fake;
mod fixture;
pub(super) mod review;
mod surface;
use compile_boundary::compile_api;
use fixture::*;
async fn soon<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
async fn paused_peer(separate: bool) {
    let f = started().await;
    install(&f, 7);
    install(&f, 8);
    f.manager.pause_chat(7).unwrap();
    let first = update(7, 41, 101, frame(0, 101, "fixture 101"));
    let second = update(8, 42, 102, frame(0, 102, "fixture 102"));
    let (tx, mut receiver) = super::test_fixture::receiver(&f.ingress, &f.pool).await;
    if separate {
        super::test_fixture::capture(&mut receiver, &tx, envelope(vec![first], 2)).await;
        super::test_fixture::capture(&mut receiver, &tx, envelope(vec![second], 3)).await;
    } else {
        super::test_fixture::capture(&mut receiver, &tx, envelope(vec![first, second], 2)).await;
    }
    f.manager.drain_ingress().await.unwrap();
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    let held = f.manager.pending_deliveries(8).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(message(&held[0]).message.random_id(), 102);
    let rows = f.ingress.pending_batches().unwrap();
    assert_eq!(rows[0].cursor, 0);
    assert_eq!(
        rows[0].applied,
        if separate {
            Default::default()
        } else {
            [1].into()
        }
    );
    assert_eq!(rows.len(), 1, "the healthy completed batch is pruned");
    let permit = f
        .ingress
        .guard_request(
            2,
            &tl::functions::messages::ReceivedQueue { max_qts: 42 }.to_bytes(),
        )
        .unwrap();
    f.ingress.cancel_request(permit);
    f.manager.resume_chat(7).await.unwrap();
    assert_eq!(
        message(&f.manager.pending_deliveries(7).unwrap()[0])
            .message
            .random_id(),
        101
    );
    assert_eq!(f.manager.pending_deliveries(8).unwrap().len(), 1);
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    f.manager.stop().await.unwrap();
}
#[tokio::test]
async fn paused_peer_does_not_block_healthy_same_batch() {
    paused_peer(false).await;
    // A creation owns its global slot before its first DH await. An unknown
    // terminal update cannot be proved unrelated until that owner has exited.
    let f = started().await;
    install(&f, 8);
    let gate = Arc::new(fake::Gate::default());
    f.world.lock().unwrap().dh_gate = Some(Arc::clone(&gate));
    let manager = f.manager.clone();
    let creation = tokio::spawn(async move { manager.create(fake::peer(2000)).await });
    soon(gate.entered.notified()).await;
    let discarded: tl::enums::Update = tl::types::UpdateEncryption {
        chat: tl::types::EncryptedChatDiscarded {
            history_deleted: false,
            id: 999,
        }
        .into(),
        date: 124,
    }
    .into();
    capture(
        &f,
        vec![
            discarded.clone(),
            update(8, 41, 101, frame(0, 101, "healthy after held create")),
        ],
        2,
    )
    .await;
    f.manager.drain_ingress().await.unwrap();
    let rows = f.ingress.pending_batches().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cursor, 0);
    assert_eq!(rows[0].applied, [1].into());
    assert_eq!(f.manager.pending_deliveries(8).unwrap().len(), 1);
    creation.abort();
    assert!(creation.await.unwrap_err().is_cancelled());
    f.manager.drain_ingress().await.unwrap();
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    f.world.lock().unwrap().dh_gate = None;
    // An uncertain cancelled request retains protected DH/request identity after
    // its active task and creation slot disappear. It also prevents this proof.
    let gate = Arc::new(fake::Gate::default());
    let wait = Arc::clone(&gate);
    f.world.lock().unwrap().request_hook = Some(Box::new(move |_| {
        Box::pin(async move {
            wait.entered.notify_one();
            wait.release.notified().await;
        })
    }));
    let manager = f.manager.clone();
    let creation = tokio::spawn(async move { manager.create(fake::peer(2000)).await });
    soon(gate.entered.notified()).await;
    creation.abort();
    assert!(creation.await.unwrap_err().is_cancelled());
    capture(
        &f,
        vec![
            discarded,
            update(8, 42, 102, frame(1, 102, "healthy after cancelled create")),
        ],
        3,
    )
    .await;
    f.manager.drain_ingress().await.unwrap();
    let rows = f.ingress.pending_batches().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cursor, 0);
    assert_eq!(rows[0].applied, [1].into());
    assert_eq!(f.manager.pending_deliveries(8).unwrap().len(), 2);
    f.manager.stop().await.unwrap();
}
#[tokio::test]
async fn paused_peer_does_not_block_healthy_separate_batches() {
    paused_peer(true).await;
}
#[tokio::test]
async fn protocol_cursor_and_consumer_identity_survive_reconstruction() {
    let f = started().await;
    install(&f, 7);
    capture(
        &f,
        vec![update(
            7,
            41,
            37,
            frame(0, 37, "committed synthetic payload"),
        )],
        2,
    )
    .await;
    f.manager.drain_ingress().await.unwrap();
    let held = f.manager.pending_deliveries(7).unwrap();
    assert_eq!(held.len(), 1);
    let before = message(&held[0]);
    assert_eq!(before.message.random_id(), 37);
    assert_eq!(f.ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    let binding = f.ingress.binding().unwrap();
    f.manager.stop().await.unwrap();
    let (resumed, ingress, _pool) = reopen(&f.dir.path().join("state/custody.bin"), binding).await;
    let after = resumed.pending_deliveries(7).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].seq_no, held[0].seq_no);
    assert_eq!(after[0].message.body, held[0].message.body);
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    resumed.acknowledge_delivery(7, after[0].seq_no).unwrap();
    assert!(resumed.pending_deliveries(7).unwrap().is_empty());
    resumed.stop().await.unwrap();
}
#[tokio::test]
async fn expired_spool_refuses_ack_without_rewriting_custody() {
    crate::custody::bounded::review::verify_refusal_ownership();
    review::expired_receipts_reopen_and_drain().await;
    let (dir, ingress, pool) = super::test_fixture::fixture(32).await;
    ingress.core.lock().unwrap().limits.age_seconds = 0.2;
    let (tx, mut receiver) = super::test_fixture::receiver(&ingress, &pool).await;
    super::test_fixture::capture(
        &mut receiver,
        &tx,
        super::test_fixture::live(41, 2, b"age fixture".to_vec()),
    )
    .await;
    let path = dir.path().join("state/custody.bin");
    let before = std::fs::read(&path).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes()
            )
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(ingress.pending_batches().unwrap().len(), 1);
}
#[tokio::test]
async fn second_manager_cannot_claim_existing_writer() {
    let f = started().await;
    assert!(f.ingress.protocol_storage().is_err());
    let client = grammers_client::Client::new(f.pool.handle.clone());
    let transport =
        crate::transport::GrammersTransport::from_verified_ingress(client, Arc::clone(&f.ingress));
    assert!(
        StrongSecretChatManager::with_transport(transport, Arc::clone(&f.ingress), budget())
            .await
            .is_err()
    );
    assert!(f.ingress.core.lock().unwrap().storage_claimed);
    assert!(!f.ingress.core.lock().unwrap().released);
    f.manager.stop().await.unwrap();
}
#[tokio::test]
async fn unprepared_original_client_cannot_construct_strong_manager() {
    let dir = tempfile::tempdir().unwrap();
    let binding =
        crate::binding::AuthorizationBinding::from_session_parts(123, 456, 2, "fixture".into())
            .unwrap();
    let ingress = DurableIngress::new(
        EncryptedIngressStorage::open(dir.path().join("state.bin"), [9; 32], binding, 1024 * 1024)
            .unwrap(),
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let pool = pool();
    let client = grammers_client::Client::new(pool.handle.clone());
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        StrongSecretChatManager::new(client, Arc::clone(&ingress), budget()),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!ingress.core.lock().unwrap().storage_claimed);
    assert!(!ingress.core.lock().unwrap().attached);
    assert!(!ingress.core.lock().unwrap().verified);
}
async fn uncertain_rekey(ordinary_first: bool) {
    let f = started().await;
    install(&f, 7);
    let dh = crate::dh::DhConfig::new(2, &fake::prime()).unwrap();
    let secret = crate::dh::SecretExponent::generate().unwrap();
    let request = crate::types::DecryptedMessageService {
        random_id: 37,
        action: crate::types::DecryptedMessageActionRequestKey {
            exchange_id: 5,
            g_a: dh.public_value(&secret),
        }
        .into(),
    };
    let mut updates = Vec::new();
    if ordinary_first {
        updates.push(update(7, 41, 36, frame(0, 36, "prior relay")));
    }
    let raw = if ordinary_first { 1 } else { 0 };
    updates.push(update(7, 41 + raw, 37, encrypt(request.into(), raw)));
    capture(&f, updates, 2).await;
    f.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    f.manager.drain_ingress().await.unwrap();
    let before = {
        let core = f.ingress.core.lock().unwrap();
        let chat = &core.state.chats[&7];
        assert!(matches!(
            chat.exchange,
            Some(crate::chat::Exchange::Accepted { .. })
        ));
        assert!(
            chat.service_deliveries
                .values()
                .all(|row| row.applied == Some(true))
        );
        let pending = chat
            .outbox
            .values()
            .filter(|row| row.pending)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(pending.len(), 1);
        pending[0].clone()
    };
    let sent = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(before.random_id, sent.outgoing.random_id);
    assert_eq!(before.frame, sent.outgoing.frame);
    let count = f.world.lock().unwrap().sent.len();
    f.manager.test_drain_deliveries(7).await.unwrap();
    let after = f.ingress.core.lock().unwrap().state.chats[&7]
        .outbox
        .values()
        .find(|row| row.pending)
        .unwrap()
        .clone();
    assert_eq!(before.random_id, after.random_id);
    assert_eq!(before.frame, after.frame);
    assert_eq!(f.world.lock().unwrap().sent.len(), count);
    assert_eq!(
        f.manager.pending_deliveries(7).unwrap().len(),
        usize::from(ordinary_first)
    );
    f.manager.stop().await.unwrap();
}
#[tokio::test]
async fn rekey_response_retains_identity_after_uncertain_rpc() {
    uncertain_rekey(false).await;
}
#[tokio::test]
async fn ordinary_relay_precedes_uncertain_rekey_response() {
    uncertain_rekey(true).await;
    replay_observes_a_wakeup_during_its_snapshot().await;
}
async fn replay_observes_a_wakeup_during_its_snapshot() {
    use std::{future::Future, task::Poll};
    let f = started().await;
    install(&f, 7);
    let dh = crate::dh::DhConfig::new(2, &fake::prime()).unwrap();
    let secret = crate::dh::SecretExponent::generate().unwrap();
    let request = crate::types::DecryptedMessageService {
        random_id: 37,
        action: crate::types::DecryptedMessageActionRequestKey {
            exchange_id: 5,
            g_a: dh.public_value(&secret),
        }
        .into(),
    };
    let (tx, mut receiver) = super::test_fixture::receiver(&f.ingress, &f.pool).await;
    super::test_fixture::capture(
        &mut receiver,
        &tx,
        envelope(vec![update(7, 41, 37, encrypt(request.into(), 0))], 2),
    )
    .await;
    let gate = Arc::new(fake::Gate::default());
    f.world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = f.manager.clone();
    let first = tokio::spawn(async move { manager.drain_ingress().await });
    soon(gate.entered.notified()).await;
    super::test_fixture::capture(
        &mut receiver,
        &tx,
        envelope(
            vec![update(7, 42, 88, frame(1, 88, "wakeup after snapshot"))],
            3,
        ),
    )
    .await;
    let manager = f.manager.clone();
    let mut joined = Box::pin(manager.drain_ingress());
    std::future::poll_fn(|cx| {
        assert!(joined.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    f.world.lock().unwrap().send_gate = None;
    gate.release.notify_one();
    soon(first).await.unwrap().unwrap();
    soon(joined).await.unwrap();
    let deliveries = f.manager.pending_deliveries(7).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(message(&deliveries[0]).message.random_id(), 88);
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    f.manager.stop().await.unwrap();
}
async fn crash_barrier(stage: u8) {
    let f = started().await;
    install(&f, 7);
    let wire = frame(0, 77, "boundary fixture");
    capture(&f, vec![update(7, 41, 77, wire.clone())], 2).await;
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    match stage {
        0 => {
            let previous = std::fs::read(&path).unwrap();
            f.ingress
                .core
                .lock()
                .unwrap()
                .backend
                .fail_next_commit(crate::storage::CommitFault::BeforeReplace);
            assert!(f.manager.drain_ingress().await.is_err());
            assert_eq!(f.ingress.pending_batches().unwrap()[0].cursor, 0);
            assert_eq!(f.ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 0);
            assert_eq!(std::fs::read(&path).unwrap(), previous);
            assert!(f.manager.stop().await.is_err());
            release_after_crash(&f.ingress);
        }
        1 => {
            let id = f.ingress.pending_batches().unwrap()[0].id;
            f.ingress
                .apply(id, 0, async { f.manager.test_receive_protocol(7, &wire) })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(f.ingress.pending_batches().unwrap()[0].cursor, 1);
            release_after_crash(&f.ingress);
        }
        _ => {
            use std::sync::atomic::{AtomicUsize, Ordering};
            let delivered = Arc::new(AtomicUsize::new(0));
            let sink = Arc::clone(&delivered);
            f.manager
                .on("MessageReceived", "consumer", move |_| {
                    sink.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
            f.manager.drain_ingress().await.unwrap();
            assert_eq!(delivered.load(Ordering::SeqCst), 1);
            assert_eq!(f.manager.pending_deliveries(7).unwrap().len(), 1);
            release_after_crash(&f.ingress);
        }
    }
    let (recovered, ingress, _pool) = reopen(&path, binding).await;
    if recovered.pause_status(7).unwrap().paused {
        assert!(recovered.pause_status(7).unwrap().resumable);
        recovered.resume_chat(7).await.unwrap();
    }
    let held = recovered.pending_deliveries(7).unwrap();
    assert_eq!(held.len(), 1);
    let wrapper = message(&held[0]);
    assert_eq!(wrapper.message.random_id(), 77);
    assert!(
        matches!(wrapper.message,crate::enums::DecryptedMessage::Message(m)if m.message=="boundary fixture")
    );
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    assert!(ingress.pending_batches().unwrap().is_empty());
    recovered.acknowledge_delivery(7, held[0].seq_no).unwrap();
    assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    recovered.stop().await.unwrap();
}
#[tokio::test]
async fn reconstruct_before_protocol_commit() {
    crash_barrier(0).await;
}
#[tokio::test]
async fn reconstruct_after_protocol_commit_before_callback() {
    crash_barrier(1).await;
}
#[tokio::test]
async fn reconstruct_after_consumer_relay_projection() {
    crash_barrier(2).await;
}
#[tokio::test]
async fn stop_joins_replay_before_unlock_and_refuses_replay_self_join() {
    let f = started().await;
    let gate = Arc::new(fake::Gate::default());
    f.world.lock().unwrap().dh_gate = Some(Arc::clone(&gate));
    requested(&f, 7).await;
    let manager = f.manager.clone();
    let replay = tokio::spawn(async move { manager.drain_ingress().await });
    soon(gate.entered.notified()).await;
    f.manager.stop().await.unwrap();
    assert!(soon(replay).await.unwrap().is_err());
    assert!(f.ingress.core.lock().unwrap().released);
    assert!(!f.ingress.core.lock().unwrap().state.chats.contains_key(&7));
    assert_eq!(f.ingress.pending_batches().unwrap()[0].cursor, 0);
    let binding = f.ingress.binding().unwrap();
    let unlocked = EncryptedIngressStorage::open(
        f.dir.path().join("state/custody.bin"),
        [9; 32],
        binding,
        1024 * 1024,
    )
    .unwrap();
    drop(unlocked);
    // A stop issued by replay itself fences ACKs but cannot release its own writer.
    let f = started().await;
    use std::sync::atomic::{AtomicBool, Ordering};
    let refused = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&refused);
    let manager = f.manager.clone();
    f.world.lock().unwrap().dh_hook = Some(Box::new(move || {
        Box::pin(async move {
            observed.store(manager.stop().await.is_err(), Ordering::SeqCst);
        })
    }));
    requested(&f, 7).await;
    assert!(f.manager.drain_ingress().await.is_err());
    assert!(refused.load(Ordering::SeqCst));
    assert!(!f.ingress.core.lock().unwrap().released);
    assert!(
        EncryptedIngressStorage::open(
            f.dir.path().join("state/custody.bin"),
            [9; 32],
            f.ingress.binding().unwrap(),
            1024 * 1024
        )
        .is_err()
    );
    f.manager.stop().await.unwrap();
    assert!(f.ingress.core.lock().unwrap().released);
}
#[tokio::test]
async fn default_client_and_public_modes_remain_explicitly_distinct() {
    surface::forwarded_operations_preserve_durable_and_file_contracts().await;
    use crate::{
        manager::{BoundedSecretChatManager, SecretChatManager, TransientSecretChatManager},
        storage::MemoryStorage,
    };
    use std::any::TypeId;
    let f = started().await;
    let (_ordinary_dir, ordinary_ingress, ordinary, baseline) = super::test_fixture::unprepared(32);
    let client = grammers_client::Client::new(ordinary.handle.clone());
    let state = fake::TestStorage::default();
    let plain = SecretChatManager::new(
        crate::transport::GrammersTransport::new(client),
        state.clone(),
        "11".repeat(32),
        crate::manager::Options::default(),
    )
    .unwrap();
    plain.stop_auto_save_secret_chats().await.unwrap();
    assert_eq!(
        state.state.lock().unwrap().as_ref().unwrap().mode,
        crate::storage::StoreMode::Durable
    );
    ordinary_ingress
        .prepare(&ordinary.handle, Some(baseline))
        .await
        .unwrap();
    assert_ne!(
        TypeId::of::<SecretChatManager<MemoryStorage, fake::Fake>>(),
        TypeId::of::<BoundedSecretChatManager<MemoryStorage, fake::Fake>>()
    );
    assert_ne!(
        TypeId::of::<Strong>(),
        TypeId::of::<TransientSecretChatManager<MemoryStorage, fake::Fake>>()
    );
    assert_eq!(crate::custody::TransientLimits::default().tasks, 8);
    assert!(f.manager.auto_save_secret_chats().is_none());
    f.manager.stop_auto_save_secret_chats().await.unwrap();
    assert!(f.manager.auto_save_secret_chats().is_none());
    install(&f, 7);
    f.manager
        .on_task("ChatReady", "native_task", |_| {
            tokio::spawn(async { Ok(()) })
        })
        .unwrap();
    f.manager
        .on_task_with_cleanup(
            "ChatReady",
            "native_cleanup",
            |_| tokio::spawn(async { Ok(()) }),
            |_| async { Ok(()) },
        )
        .unwrap();
    assert_eq!(f.manager.active_handler_tasks(), 0);
    f.manager
        .delete_messages(7, std::iter::once(101))
        .await
        .unwrap();
    f.manager.flush_history(7).await.unwrap();
    assert!(!f.manager.pause_status(7).unwrap().paused);
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    f.manager.stop().await.unwrap();
}
#[test]
fn plain_and_volatile_storage_are_rejected_at_rust_type_boundary() {
    let compiled = compile_api(
        "use kiaro_secret_chat::{ingress::{DurableIngress,EncryptedIngressStorage},custody::IngressLimits};fn valid(store:EncryptedIngressStorage,limits:IngressLimits){let _=DurableIngress::new(store,limits);}",
    );
    assert!(
        compiled.status.success(),
        "native API metadata canary must typecheck: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for backend in ["MemoryStorage", "FileStorage"] {
        let source = format!(
            "use kiaro_secret_chat::{{ingress::DurableIngress,custody::IngressLimits,storage::{backend}}};fn invalid(store:{backend},limits:IngressLimits){{let _=DurableIngress::new(store,limits);}}"
        );
        let output = compile_api(&source);
        assert!(
            !output.status.success(),
            "plain or volatile storage must not typecheck as strong custody"
        );
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        assert!(diagnostic.contains("E0308"));
        assert!(diagnostic.contains("EncryptedIngressStorage"));
        assert!(diagnostic.contains(backend));
    }
}
