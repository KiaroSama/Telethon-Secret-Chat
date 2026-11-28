//! Readiness transitions must revisit durable input without another external wakeup.
use super::super::test_fixture;
use super::{fake, fixture::*, soon, tl};
use crate::{
    Serializable,
    chat::{Chat, ChatState, DhRecord},
    crypto::{Direction, encrypt_frame},
    custody::RelayDelivery,
    dh::{DhConfig, SecretExponent},
    error::{Error, PendingCause},
    manager::{Event, RpcFailure},
    protocol::SendEffect,
    types::DecryptedMessage,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn install_waiting(f: &Fixture) -> (tl::enums::Update, tl::enums::Update) {
    let prime = fake::prime();
    let dh = DhConfig::new(2, &prime).unwrap();
    let local = SecretExponent::from_bytes([0x81; 256]);
    let remote = SecretExponent::from_bytes([0x82; 256]);
    let peer_public = dh.public_value(&remote);
    let key = dh.shared_key(&peer_public, &local).unwrap();
    let mut waiting = Chat::new(7, 88, 2000, false, ChatState::Waiting);
    waiting.handshake_secret = Some(local);
    waiting.peer_public = Some(peer_public.clone());
    waiting.dh = Some(DhRecord { g: 2, prime });
    waiting.admin_id = Some(2000);
    waiting.participant_id = Some(1000);
    f.manager.test_install_chat(waiting).unwrap();
    f.world
        .lock()
        .unwrap()
        .requests
        .insert(7, (2000, 1000, peer_public.clone()));

    let body = DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 91,
        ttl: 0,
        message: "retained until establishment completes".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    };
    let wrapper = crate::framing::wrap(body.into(), 144, 0, 1).unwrap();
    let wire = encrypt_frame(&key, &wrapper.to_bytes(), Direction::Initiator).unwrap();
    let established = tl::types::UpdateEncryption {
        chat: tl::types::EncryptedChat {
            id: 7,
            access_hash: 88,
            date: 124,
            admin_id: 2000,
            participant_id: 1000,
            g_a_or_b: peer_public,
            key_fingerprint: key.fingerprint(),
        }
        .into(),
        date: 124,
    }
    .into();
    (update(7, 41, 91, wire), established)
}

fn one_receipt(manager: &Strong) -> RelayDelivery {
    assert_eq!(manager.status(7).unwrap().state, ChatState::Ready);
    let mut held = manager.pending_deliveries(7).unwrap();
    assert_eq!(held.len(), 1, "ready input needs no extra external wakeup");
    assert_eq!(held[0].seq_no, 1);
    assert_eq!(message(&held[0]).message.random_id(), 91);
    held.remove(0)
}

async fn check_repeat_and_restart(f: &Fixture) {
    let before = one_receipt(&f.manager);
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    soon(f.manager.drain_ingress()).await.unwrap();
    let repeated = one_receipt(&f.manager);
    assert_eq!(repeated.seq_no, before.seq_no);
    assert_eq!(repeated.message.body, before.message.body);
    assert_eq!(f.ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding.clone())).await;
    let restored = one_receipt(&recovered);
    assert_eq!(restored.seq_no, before.seq_no);
    assert_eq!(restored.message.body, before.message.body);
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    assert!(ingress.pending_batches().unwrap().is_empty());
    soon(recovered.drain_ingress()).await.unwrap();
    assert_eq!(one_receipt(&recovered).seq_no, before.seq_no);
    recovered.acknowledge_delivery(7, restored.seq_no).unwrap();
    soon(recovered.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding)).await;
    assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    soon(recovered.stop()).await.unwrap();
}

async fn establishment_revisits_deferred_message(same_batch: bool) {
    let f = started().await;
    let (held, established) = install_waiting(&f);
    let (tx, mut receiver) = test_fixture::receiver(&f.ingress, &f.pool).await;
    if same_batch {
        // The SDK sorts establishment before qts-bearing messages in this envelope.
        test_fixture::capture(&mut receiver, &tx, envelope(vec![held, established], 2)).await;
    } else {
        test_fixture::capture(&mut receiver, &tx, envelope(vec![held], 2)).await;
        soon(f.manager.drain_ingress()).await.unwrap();
        assert_eq!(f.manager.status(7).unwrap().state, ChatState::Waiting);
        assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
        test_fixture::capture(&mut receiver, &tx, envelope(vec![established], 3)).await;
    }
    soon(f.manager.drain_ingress()).await.unwrap();
    one_receipt(&f.manager);
    check_repeat_and_restart(&f).await;
}

#[tokio::test]
async fn one_drain_revisits_message_before_same_batch_establishment() {
    establishment_revisits_deferred_message(true).await;
}

#[tokio::test]
async fn one_drain_revisits_message_before_later_batch_establishment() {
    establishment_revisits_deferred_message(false).await;
}

#[tokio::test]
async fn startup_revisits_message_when_establishment_is_already_durable() {
    let f = started().await;
    let (held, established) = install_waiting(&f);
    let (tx, mut receiver) = test_fixture::receiver(&f.ingress, &f.pool).await;
    test_fixture::capture(&mut receiver, &tx, envelope(vec![held], 2)).await;
    test_fixture::capture(&mut receiver, &tx, envelope(vec![established], 3)).await;
    assert_eq!(f.ingress.pending_batches().unwrap().len(), 2);
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding.clone())).await;
    let before = one_receipt(&recovered);
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    soon(recovered.drain_ingress()).await.unwrap();
    assert_eq!(one_receipt(&recovered).seq_no, before.seq_no);
    soon(recovered.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding)).await;
    let restored = one_receipt(&recovered);
    assert_eq!(restored.seq_no, before.seq_no);
    assert_eq!(restored.message.body, before.message.body);
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert_eq!(ingress.core.lock().unwrap().state.chats[&7].in_seq_no, 1);
    recovered.acknowledge_delivery(7, restored.seq_no).unwrap();
    soon(recovered.stop()).await.unwrap();
}

#[tokio::test]
async fn accepting_waiting_chat_revisits_input_without_another_update() {
    let f = started().await;
    let (held, _) = install_waiting(&f);
    capture(&f, vec![held], 2).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    assert_eq!(f.ingress.pending_batches().unwrap()[0].cursor, 0);
    // The remote acceptance completed before local recovery of its persisted secret.
    f.world.lock().unwrap().fail_accept = Some(RpcFailure::AlreadyAccepted);
    let ready = soon(f.manager.accept(7)).await.unwrap();
    assert_eq!(ready.state, ChatState::Ready);
    one_receipt(&f.manager);
    check_repeat_and_restart(&f).await;
}

#[tokio::test]
async fn replay_without_progress_preserves_waiting_and_paused_rows() {
    let f = started().await;
    let (held, _) = install_waiting(&f);
    install(&f, 8);
    install(&f, 9);
    f.manager.pause_chat(8).unwrap();
    capture(
        &f,
        vec![
            held,
            update(8, 42, 92, frame(0, 92, "paused input")),
            update(9, 43, 93, frame(0, 93, "healthy input")),
        ],
        2,
    )
    .await;
    for _ in 0..2 {
        soon(f.manager.drain_ingress()).await.unwrap();
        assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
        assert!(f.manager.pending_deliveries(8).unwrap().is_empty());
        let healthy = f.manager.pending_deliveries(9).unwrap();
        assert_eq!(healthy.len(), 1);
        assert_eq!(message(&healthy[0]).message.random_id(), 93);
        let batches = f.ingress.pending_batches().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].cursor, 0);
        assert_eq!(batches[0].applied, [2].into());
    }
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();
    let (recovered, ingress, _pool) = soon(reopen(&path, binding)).await;
    assert_eq!(recovered.status(7).unwrap().state, ChatState::Waiting);
    assert!(recovered.pause_status(8).unwrap().paused);
    assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    assert!(recovered.pending_deliveries(8).unwrap().is_empty());
    let healthy = recovered.pending_deliveries(9).unwrap();
    assert_eq!(healthy.len(), 1);
    assert_eq!(message(&healthy[0]).message.random_id(), 93);
    let batches = ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].cursor, 0);
    assert_eq!(batches[0].applied, [2].into());
    {
        let core = ingress.core.lock().unwrap();
        assert_eq!(core.state.chats[&7].in_seq_no, 0);
        assert_eq!(core.state.chats[&8].in_seq_no, 0);
        assert_eq!(core.state.chats[&9].in_seq_no, 1);
    }
    soon(recovered.stop()).await.unwrap();
}

#[tokio::test]
async fn accept_reentered_from_replay_does_not_wait_for_its_own_task() {
    let f = started().await;
    let _ = install_waiting(&f);
    let accepted = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&accepted);
    let manager = f.manager.clone();
    {
        let mut world = f.world.lock().unwrap();
        world.fail_accept = Some(RpcFailure::AlreadyAccepted);
        // This transport hook runs inline in the task replaying the other request.
        world.dh_hook = Some(Box::new(move || {
            Box::pin(async move {
                let ready = manager.accept(7).await.unwrap();
                assert_eq!(ready.state, ChatState::Ready);
                observed.store(true, Ordering::SeqCst);
            })
        }));
    }
    requested(&f, 9).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    assert!(accepted.load(Ordering::SeqCst));
    assert_eq!(f.manager.status(7).unwrap().state, ChatState::Ready);
    assert_eq!(f.manager.status(9).unwrap().state, ChatState::Waiting);
    assert!(f.ingress.pending_batches().unwrap().is_empty());
    soon(f.manager.stop()).await.unwrap();
}

fn observe_readiness_delivery(f: &Fixture) -> Arc<tokio::sync::Notify> {
    let delivered = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::clone(&delivered);
    f.manager
        .on("MessageReceived", "readiness_delivery", move |event| {
            if matches!(event, Event::MessageReceived(message)
                if message.chat_id == 7 && message.random_id == 91)
            {
                observed.notify_one();
            }
            Ok(())
        })
        .unwrap();
    delivered
}

async fn await_owned_readiness_replay(f: &Fixture, delivered: &tokio::sync::Notify) {
    soon(delivered.notified()).await;
    // Observe the owned replay finishing; do not provide a new drain wakeup.
    await_empty_ingress(f).await;
    one_receipt(&f.manager);
}

async fn await_empty_ingress(f: &Fixture) {
    soon(async {
        while !f.ingress.pending_batches().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

fn assert_initial_layer_still_pending(f: &Fixture, attempt: &SendEffect) {
    assert_eq!(f.manager.status(7).unwrap().state, ChatState::Ready);
    assert!(!f.manager.pause_status(7).unwrap().paused);
    let core = f.ingress.core.lock().unwrap();
    let retained = core.state.chats[&7]
        .outbox
        .get(&attempt.outgoing.seq_no)
        .unwrap();
    assert!(retained.pending);
    assert!(retained.service);
    assert_eq!(retained.random_id, attempt.outgoing.random_id);
    assert_eq!(retained.frame, attempt.outgoing.frame);
}

#[tokio::test]
async fn acceptance_send_error_keeps_error_and_wakes_ready_input() {
    let f = started().await;
    let (held, _) = install_waiting(&f);
    capture(&f, vec![held], 2).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    let delivered = observe_readiness_delivery(&f);
    {
        let mut world = f.world.lock().unwrap();
        world.fail_accept = Some(RpcFailure::AlreadyAccepted);
        world.fail_send = Some(RpcFailure::Transient);
    }
    let result = soon(f.manager.accept(7)).await;
    let attempt = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(
        result.unwrap_err(),
        Error::SendPending {
            chat_id: 7,
            random_id: attempt.outgoing.random_id,
            cause: PendingCause::Transport,
        }
    );
    assert_initial_layer_still_pending(&f, &attempt);
    await_owned_readiness_replay(&f, &delivered).await;
    assert_initial_layer_still_pending(&f, &attempt);
    check_repeat_and_restart(&f).await;
}

#[tokio::test]
async fn cancelled_acceptance_wakes_ready_input_and_keeps_pending_send() {
    let f = started().await;
    let (held, _) = install_waiting(&f);
    capture(&f, vec![held], 2).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    let delivered = observe_readiness_delivery(&f);
    let gate = Arc::new(fake::Gate::default());
    {
        let mut world = f.world.lock().unwrap();
        world.fail_accept = Some(RpcFailure::AlreadyAccepted);
        world.send_gate = Some(Arc::clone(&gate));
    }
    let manager = f.manager.clone();
    let accepting = tokio::spawn(async move { manager.accept(7).await });
    soon(gate.entered.notified()).await;
    let attempt = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_initial_layer_still_pending(&f, &attempt);
    // The current send owns its gate; later retry work must not inherit it.
    f.world.lock().unwrap().send_gate = None;
    accepting.abort();
    assert!(soon(accepting).await.unwrap_err().is_cancelled());
    assert_eq!(f.world.lock().unwrap().active_sends, 0);
    assert_initial_layer_still_pending(&f, &attempt);
    await_owned_readiness_replay(&f, &delivered).await;
    assert_initial_layer_still_pending(&f, &attempt);
    check_repeat_and_restart(&f).await;
}

async fn creation_admission_retires_held_absent_row(cancel: bool) {
    let f = started().await;
    let gate = Arc::new(fake::Gate::default());
    f.world.lock().unwrap().dh_gate = Some(Arc::clone(&gate));
    let manager = f.manager.clone();
    let creating = tokio::spawn(async move { manager.create(fake::peer(2000)).await });
    soon(gate.entered.notified()).await;
    capture(&f, vec![update(999, 41, 94, vec![])], 2).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    let batches = f.ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].cursor, 0);
    assert!(batches[0].applied.is_empty());
    {
        let core = f.ingress.core.lock().unwrap();
        assert!(core.state.chats.is_empty());
        assert!(
            core.state
                .private_records
                .keys()
                .all(|name| !name.starts_with("pending-create/"))
        );
    }
    f.world.lock().unwrap().dh_gate = None;
    if cancel {
        creating.abort();
        assert!(soon(creating).await.unwrap_err().is_cancelled());
    } else {
        gate.release.notify_one();
        let created = soon(creating).await.unwrap().unwrap();
        assert_eq!(created.id, 7);
        assert_eq!(created.state, ChatState::Requested);
    }
    // Completion or cancellation must release admission and own the retirement wake.
    await_empty_ingress(&f).await;
    assert!(f.manager.status(999).is_err());
    {
        let core = f.ingress.core.lock().unwrap();
        assert!(
            core.state
                .private_records
                .keys()
                .all(|name| !name.starts_with("pending-create/"))
        );
        if cancel {
            assert!(core.state.chats.is_empty());
        } else {
            assert_eq!(core.state.chats.len(), 1);
            assert_eq!(core.state.chats[&7].state, ChatState::Requested);
            assert_eq!(core.state.chats[&7].in_seq_no, 0);
        }
    }
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();
    let (recovered, ingress, _pool) = soon(reopen(&path, binding)).await;
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert!(recovered.status(999).is_err());
    if cancel {
        assert!(recovered.list().unwrap().is_empty());
    } else {
        assert_eq!(recovered.status(7).unwrap().state, ChatState::Requested);
        assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    }
    soon(recovered.stop()).await.unwrap();
}

#[tokio::test]
async fn completed_creation_retires_absent_row_held_by_admission() {
    creation_admission_retires_held_absent_row(false).await;
}

#[tokio::test]
async fn cancelled_creation_retires_absent_row_after_admission_drops() {
    creation_admission_retires_held_absent_row(true).await;
}

#[test]
fn acceptance_dropped_outside_runtime_wakes_input_after_reentry() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let f = runtime.block_on(started());
    let (held, _) = install_waiting(&f);
    runtime.block_on(async {
        capture(&f, vec![held], 2).await;
        soon(f.manager.drain_ingress()).await.unwrap();
    });
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    let delivered = observe_readiness_delivery(&f);
    let gate = Arc::new(fake::Gate::default());
    {
        let mut world = f.world.lock().unwrap();
        world.fail_accept = Some(RpcFailure::AlreadyAccepted);
        world.send_gate = Some(Arc::clone(&gate));
    }
    let manager = f.manager.clone();
    let mut accepting = Box::pin(async move { manager.accept(7).await });
    runtime.block_on(async {
        soon(async {
            tokio::select! {
                _ = accepting.as_mut() => panic!("gated acceptance unexpectedly completed"),
                _ = gate.entered.notified() => {}
            }
        })
        .await;
    });
    let attempt = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_initial_layer_still_pending(&f, &attempt);
    f.world.lock().unwrap().send_gate = None;
    // Cancellation occurs with no runtime entered; the guard retains its original handle.
    assert!(tokio::runtime::Handle::try_current().is_err());
    drop(accepting);
    assert_eq!(f.world.lock().unwrap().active_sends, 0);
    assert_initial_layer_still_pending(&f, &attempt);
    runtime.block_on(async {
        await_owned_readiness_replay(&f, &delivered).await;
        assert_initial_layer_still_pending(&f, &attempt);
        check_repeat_and_restart(&f).await;
    });
}

async fn paused_ready_with_retained_input() -> (Fixture, SendEffect, Arc<tokio::sync::Notify>) {
    let f = started().await;
    let (held, _) = install_waiting(&f);
    {
        let mut world = f.world.lock().unwrap();
        world.fail_accept = Some(RpcFailure::AlreadyAccepted);
        world.fail_send = Some(RpcFailure::Transient);
    }
    let error = soon(f.manager.accept(7)).await.unwrap_err();
    let attempt = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(
        error,
        Error::SendPending {
            chat_id: 7,
            random_id: attempt.outgoing.random_id,
            cause: PendingCause::Transport,
        }
    );
    assert_initial_layer_still_pending(&f, &attempt);
    f.manager.pause_chat(7).unwrap();
    capture(&f, vec![held], 2).await;
    soon(f.manager.drain_ingress()).await.unwrap();
    assert!(f.manager.pause_status(7).unwrap().paused);
    assert!(f.manager.pending_deliveries(7).unwrap().is_empty());
    let batches = f.ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].cursor, 0);
    assert!(batches[0].applied.is_empty());
    let delivered = observe_readiness_delivery(&f);
    (f, attempt, delivered)
}

#[tokio::test]
async fn resume_send_error_keeps_error_and_wakes_unpaused_input() {
    let (f, attempt, delivered) = paused_ready_with_retained_input().await;
    f.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    let error = soon(f.manager.resume_chat(7)).await.unwrap_err();
    assert_eq!(
        error,
        Error::SendPending {
            chat_id: 7,
            random_id: attempt.outgoing.random_id,
            cause: PendingCause::Transport,
        }
    );
    let retried = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(retried.outgoing.random_id, attempt.outgoing.random_id);
    assert_eq!(retried.outgoing.frame, attempt.outgoing.frame);
    assert_initial_layer_still_pending(&f, &attempt);
    await_owned_readiness_replay(&f, &delivered).await;
    assert_initial_layer_still_pending(&f, &attempt);
    check_repeat_and_restart(&f).await;
}

#[tokio::test]
async fn cancelled_resume_wakes_unpaused_input_and_keeps_pending_send() {
    let (f, attempt, delivered) = paused_ready_with_retained_input().await;
    let gate = Arc::new(fake::Gate::default());
    f.world.lock().unwrap().send_gate = Some(Arc::clone(&gate));
    let manager = f.manager.clone();
    let resuming = tokio::spawn(async move { manager.resume_chat(7).await });
    soon(gate.entered.notified()).await;
    let retried = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(retried.outgoing.random_id, attempt.outgoing.random_id);
    assert_eq!(retried.outgoing.frame, attempt.outgoing.frame);
    assert_initial_layer_still_pending(&f, &attempt);
    f.world.lock().unwrap().send_gate = None;
    resuming.abort();
    assert!(soon(resuming).await.unwrap_err().is_cancelled());
    assert_eq!(f.world.lock().unwrap().active_sends, 0);
    assert_initial_layer_still_pending(&f, &attempt);
    await_owned_readiness_replay(&f, &delivered).await;
    assert_initial_layer_still_pending(&f, &attempt);
    check_repeat_and_restart(&f).await;
}
