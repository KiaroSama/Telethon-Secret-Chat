//! Offline release-boundary tests. Credentials are burned deterministic fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::{custody::IngressLimits, storage::Storage};
use grammers_mtsender::DurableIngressHook;
use grammers_session::updates::MessageBoxes;
use grammers_tl_types::{self as tl, Serializable};
use std::{sync::Arc, time::Duration};

pub(super) use super::test_fixture::{capture, fixture, live, receiver};
use super::test_fixture::{difference, request};
#[tokio::test]
async fn real_release_live_receipt_precedes_ack_frontier() {
    protocol_counters_mailbox_and_applied_cursor_share_the_actual_commit().await;
    durable_raw_ingress_survives_absent_volatile_forwarding().await;

    let (_dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let bytes = tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes();
    assert!(ingress.guard_request(2, &bytes).is_err());
    capture(
        &mut receiver,
        &tx,
        live(41, 2, b"complete original envelope".to_vec()),
    )
    .await;
    let pending = ingress.pending_batches().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0]
            .envelope
            .as_bytes()
            .windows(26)
            .any(|v| v == b"complete original envelope")
    );
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        41
    );
    let permit = ingress.guard_request(2, &bytes).unwrap();
    ingress.cancel_request(permit);
}
#[tokio::test]
async fn caller_difference_is_captured_before_result_and_delivered_by_existing_receiver() {
    super::strong_tests::review::stale_difference_retries_at_current_pts().await;
    let (_dir, ingress, pool) = fixture(32).await;
    let (_tx, mut receiver) = receiver(&ingress, &pool).await;
    let request = request(40).to_bytes();
    let response = difference(42).to_bytes();
    let permit = ingress.guard_request(2, &request).unwrap();
    ingress
        .guard_result(permit, 2, &request, &response)
        .unwrap();
    let batch = ingress.pending_batches().unwrap().remove(0);
    assert_eq!(batch.envelope.as_bytes(), response);
    assert_eq!(batch.updates().unwrap().0.len(), 2);
    let result = tokio::time::timeout(Duration::from_secs(1), receiver.next()).await;
    assert!(
        result.is_ok(),
        "committed caller difference must wake the existing receiver"
    );
    assert_eq!(result.unwrap().unwrap().0.len(), 2);
}
#[tokio::test]
async fn full_spool_preserves_first_receipt_and_fences_even_old_ack() {
    let (_dir, ingress, pool) = fixture(1).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    capture(
        &mut receiver,
        &tx,
        live(41, 2, b"first held receipt".to_vec()),
    )
    .await;
    let before = ingress.checkpoint().unwrap();
    let held = ingress.pending_batches().unwrap()[0]
        .envelope
        .as_bytes()
        .to_vec();
    tx.send(live(42, 3, b"refused next receipt".to_vec()))
        .await
        .unwrap();
    assert!(receiver.next().await.is_err());
    assert_eq!(ingress.checkpoint().unwrap(), before);
    assert_eq!(
        ingress.pending_batches().unwrap()[0].envelope.as_bytes(),
        held
    );
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes()
            )
            .is_err()
    );
    assert!(ingress.guard_request(2, &request(41).to_bytes()).is_err());

    let (_dir, ingress, _pool) = fixture(32).await;
    assert!(
        ingress
            .stage_live(2, &live(41, 2, vec![0; 5 * 1024 * 1024]))
            .is_err()
    );
    assert!(
        ingress.guard_request(2, &request(40).to_bytes()).is_err(),
        "an unjournalled oversized wire envelope must fence even the old acknowledgment frontier"
    );
}
#[tokio::test]
async fn wrapped_implicit_ack_cannot_bypass_frontier() {
    unrelated_setting_and_failed_decrypt_do_not_settle_ingress().await;
    let (_dir, ingress, _pool) = fixture(32).await;
    let wrapped = tl::functions::InvokeWithoutUpdates {
        query: tl::functions::InvokeWithLayer {
            layer: 227,
            query: request(41),
        },
    }
    .to_bytes();
    assert!(ingress.guard_request(2, &wrapped).is_err());
    let mut unknown_flags = request(40).to_bytes();
    unknown_flags[4..8].copy_from_slice(&(1u32 << 31).to_le_bytes());
    assert!(
        ingress.guard_request(2, &unknown_flags).is_err(),
        "unknown schema flags cannot conceal a different server-side qts layout"
    );
    let mut unknown_wrapper = 0xdeadbeefu32.to_le_bytes().to_vec();
    unknown_wrapper.extend_from_slice(&request(41).to_bytes());
    assert!(
        ingress.guard_request(2, &unknown_wrapper).is_err(),
        "an unknown generic wrapper cannot bypass acknowledgment inspection"
    );
    let mut packed = 0x3072cfa1u32.to_le_bytes().to_vec();
    packed.extend_from_slice(&[0, 0, 0, 0]);
    assert!(
        ingress.guard_request(2, &packed).is_err(),
        "a caller-provided MTProto packed envelope cannot hide an acknowledgment"
    );
    let mut init = tl::functions::InitConnection {
        api_id: 123,
        device_model: "public fixture".into(),
        system_version: "fixture".into(),
        app_version: "fixture".into(),
        system_lang_code: "en".into(),
        lang_pack: String::new(),
        lang_code: "en".into(),
        proxy: None,
        params: None,
        query: request(40),
    }
    .to_bytes();
    init[4..8].copy_from_slice(&(1u32 << 31).to_le_bytes());
    assert!(ingress.guard_request(2, &init).is_err());
    assert!(ingress.guard_request(2, &request(40).to_bytes()).is_ok());
}
async fn unissued_canceled_and_post_stop_results_cannot_capture() {
    let (_dir, ingress, _pool) = fixture(32).await;
    let request = request(40).to_bytes();
    let response = difference(42).to_bytes();
    assert!(ingress.guard_result(999, 2, &request, &response).is_err());
    let permit = ingress.guard_request(2, &request).unwrap();
    ingress.cancel_request(permit);
    assert!(
        ingress
            .guard_result(permit, 2, &request, &response)
            .is_err()
    );
    let permit = ingress.guard_request(2, &request).unwrap();
    ingress.fence().unwrap();
    assert!(
        ingress
            .guard_result(permit, 2, &request, &response)
            .is_err()
    );
    assert!(ingress.pending_batches().unwrap().is_empty());
}
#[tokio::test]
async fn public_preparation_requires_same_client_proof_before_any_ack() {
    canceled_same_client_proof_can_be_retried_without_ack_permission().await;
    a_different_original_client_cannot_complete_the_pool_proof().await;

    let (_dir, ingress, pool) = fixture(32).await;
    ingress.core.lock().unwrap().verified = false;
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 40 }.to_bytes()
            )
            .is_err()
    );
    assert!(ingress.prepare(&pool.handle, None).await.is_err());
    assert!(
        ingress
            .guard_request(2, &tl::functions::updates::GetState {}.to_bytes())
            .is_err()
    );
    ingress.fence().unwrap();
    assert!(ingress.guard_request(2, &request(40).to_bytes()).is_err());
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        40
    );
}
#[tokio::test]
async fn caller_difference_wakes_receiver_already_waiting_for_live_input() {
    let (_dir, ingress, pool) = fixture(32).await;
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let mut receive = Box::pin(receiver.next());
    std::future::poll_fn(|cx| {
        assert!(receive.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let request = request(40).to_bytes();
    let response = difference(42).to_bytes();
    let permit = ingress.guard_request(2, &request).unwrap();
    ingress
        .guard_result(permit, 2, &request, &response)
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), receive)
        .await
        .expect("durable RPC receipt must wake the original update owner")
        .expect("the original receiver must deliver the committed difference");
    assert_eq!(received.0.len(), 2);
    assert_eq!(
        received
            .0
            .iter()
            .map(|(update, _)| match update {
                tl::enums::Update::NewEncryptedMessage(update) => update.message.random_id(),
                _ => panic!("expected the two committed encrypted difference messages"),
            })
            .collect::<Vec<_>>(),
        [51, 52]
    );
    capture(
        &mut receiver,
        &tx,
        live(43, 3, b"live body after caller difference".to_vec()),
    )
    .await;
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        43
    );
}
#[tokio::test]
async fn dropping_real_original_handle_rpc_cancels_its_ingress_permit() {
    unissued_canceled_and_post_stop_results_cannot_capture().await;

    let (_dir, ingress, pool) = fixture(32).await;
    for _ in 0..65 {
        let mut call = Box::pin(pool.handle.raw_invoke_in_dc(2, request(40).to_bytes()));
        std::future::poll_fn(|cx| {
            assert!(
                call.as_mut().poll(cx).is_pending(),
                "canceled permits must not exhaust active request capacity"
            );
            std::task::Poll::Ready(())
        })
        .await;
        drop(call);
    }
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        40
    );
    assert!(ingress.guard_request(2, &request(41).to_bytes()).is_err());
}
fn protocol(
    ingress: &Arc<DurableIngress>,
) -> crate::protocol::Protocol<crate::custody::CustodyStorage<IngressProtocolStorage>> {
    use crate::{
        chat::{Chat, ChatState, DhRecord},
        crypto::SharedKey,
        custody::{CustodyLimits, CustodyStorage},
        protocol::Protocol,
    };
    let auth = ingress.binding().unwrap().digest();
    let store = CustodyStorage::new(
        ingress.protocol_storage().unwrap(),
        auth.clone(),
        CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let mut protocol = Protocol::open(store, auth).unwrap();
    protocol.start().unwrap();
    let mut chat = Chat::new(7, 11, 123, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: hex::decode(include_str!("../../tests/fixtures/dh-prime.hex").trim()).unwrap(),
    });
    protocol.insert_chat(chat).unwrap();
    protocol
}
fn frame() -> Vec<u8> {
    use crate::{
        crypto::{Direction, SharedKey, encrypt_frame},
        framing, types,
    };
    let message = types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 777,
        ttl: 0,
        message: "atomic plaintext fixture".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    };
    encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &framing::wrap(message.into(), 144, 1, 0).unwrap().to_bytes(),
        Direction::Responder,
    )
    .unwrap()
}
async fn protocol_counters_mailbox_and_applied_cursor_share_the_actual_commit() {
    let (_dir, ingress, pool) = fixture(32).await;
    let mut protocol = protocol(&ingress);
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let frame = frame();
    capture(&mut receiver, &tx, live(41, 2, frame.clone())).await;
    let batch = ingress.pending_batches().unwrap()[0].id;
    ingress
        .apply(batch, 0, async { protocol.receive(7, &frame, None, 124) })
        .await
        .unwrap()
        .unwrap();
    let core = ingress.core.lock().unwrap();
    let actual = core.backend.load().unwrap().unwrap();
    assert_eq!(actual.chats[&7].in_seq_no, 1);
    assert_eq!(actual.chats[&7].deliveries.len(), 1);
    let held = super::ledger::Ledger::load(&actual, &core.limits).unwrap();
    assert_eq!(held.batches[0].cursor, 1);
    assert_eq!(
        held.batches[0].applied.iter().copied().collect::<Vec<_>>(),
        vec![0]
    );
}
async fn unrelated_setting_and_failed_decrypt_do_not_settle_ingress() {
    let (_dir, ingress, pool) = fixture(32).await;
    let mut protocol = protocol(&ingress);
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    capture(&mut receiver, &tx, live(41, 2, frame())).await;
    let batch = ingress.pending_batches().unwrap()[0].id;
    assert!(
        ingress
            .apply(batch, 0, async {
                protocol
                    .set_setting("unrelated".into(), Some(7.into()))
                    .unwrap();
                protocol.receive(7, b"invalid frame", None, 124)
            })
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(ingress.pending_batches().unwrap()[0].cursor, 0);
    assert_eq!(protocol.chat_record(7).unwrap().in_seq_no, 0);
    assert_eq!(protocol.setting("unrelated"), Some(7.into()));
}
async fn failed_protocol_write_preserves_exact_receipt_for_reconstruction() {
    let (dir, ingress, pool) = fixture(32).await;
    let binding = ingress.binding().unwrap();
    let mut protocol = protocol(&ingress);
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let frame = frame();
    capture(&mut receiver, &tx, live(41, 2, frame.clone())).await;
    let batch = ingress.pending_batches().unwrap()[0].id;
    let original = ingress.pending_batches().unwrap()[0]
        .envelope
        .as_bytes()
        .to_vec();
    let path = dir.path().join("state/custody.bin");
    let previous = std::fs::read(&path).unwrap();
    ingress
        .core
        .lock()
        .unwrap()
        .backend
        .fail_next_commit(crate::storage::CommitFault::BeforeReplace);
    assert!(
        ingress
            .apply(batch, 0, async { protocol.receive(7, &frame, None, 124) })
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(ingress.pending_batches().unwrap()[0].cursor, 0);
    assert_eq!(protocol.chat_record(7).unwrap().in_seq_no, 0);
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    drop(receiver);
    drop(protocol);
    drop(pool);
    drop(ingress);
    let restored = EncryptedIngressStorage::open(path, [9; 32], binding, 1024 * 1024).unwrap();
    let restored = DurableIngress::new(
        restored,
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        restored.pending_batches().unwrap()[0].envelope.as_bytes(),
        original
    );
    assert_eq!(restored.pending_batches().unwrap()[0].cursor, 0);
}
async fn canceled_same_client_proof_can_be_retried_without_ack_permission() {
    let (_dir, ingress, pool) = fixture(32).await;
    ingress.core.lock().unwrap().verified = false;
    let client = grammers_client::Client::new(pool.handle.clone());
    for _ in 0..2 {
        let mut proof = Box::pin(ingress.verify_client(&client));
        std::future::poll_fn(|cx| {
            assert!(
                proof.as_mut().poll(cx).is_pending(),
                "canceled proof must release its exclusive attempt"
            );
            std::task::Poll::Ready(())
        })
        .await;
        drop(proof);
        assert!(
            ingress
                .guard_request(
                    2,
                    &tl::functions::messages::ReceivedQueue { max_qts: 40 }.to_bytes()
                )
                .is_err()
        );
    }
}
async fn a_different_original_client_cannot_complete_the_pool_proof() {
    let (_dir, ingress, _pool) = fixture(32).await;
    ingress.core.lock().unwrap().verified = false;
    let wrong = super::test_fixture::pool();
    let client = grammers_client::Client::new(wrong.handle);
    let binding = ingress.binding().unwrap();
    assert_eq!(
        client.authorization_key_tag().unwrap(),
        binding.auth_key_tag().map(|tag| (binding.dc_id(), tag))
    );
    let mut proof = Box::pin(ingress.verify_client(&client));
    std::future::poll_fn(|cx| {
        assert!(proof.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(
        ingress
            .core
            .lock()
            .unwrap()
            .proof
            .is_some_and(|(_, seen)| !seen)
    );
    drop(wrong.runner);
    drop(wrong.updates);
    assert!(proof.await.is_err());
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 40 }.to_bytes()
            )
            .is_err()
    );
}
async fn identity_change(change: u8) {
    let (_dir, ingress, pool) = fixture(32).await;
    match change {
        0 => {
            let mut dc = pool.handle.session.dc_option(2).unwrap().unwrap();
            dc.auth_key = Some([0; 256]);
            pool.handle.session.set_dc_option(&dc).await.unwrap();
        }
        1 => pool.handle.session.set_home_dc_id(3).await.unwrap(),
        _ => {
            let mut store = ingress.protocol_storage().unwrap();
            store.suspend_storage().unwrap();
        }
    }
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 40 }.to_bytes()
            )
            .is_err()
    );
}
#[tokio::test]
async fn changed_authorization_refuses_ack_before_send() {
    identity_change(0).await
}
#[tokio::test]
async fn changed_dc_refuses_ack_before_send() {
    identity_change(1).await
}
#[tokio::test]
async fn released_owner_refuses_ack_before_send() {
    identity_change(2).await
}
#[tokio::test]
async fn too_long_difference_fences_without_replacing_held_frontier() {
    let (_dir, ingress, _pool) = fixture(32).await;
    let before = ingress.checkpoint().unwrap();
    let request = request(40).to_bytes();
    let response: tl::enums::updates::Difference =
        tl::types::updates::DifferenceTooLong { pts: 100 }.into();
    let permit = ingress.guard_request(2, &request).unwrap();
    assert!(
        ingress
            .guard_result(permit, 2, &request, &response.to_bytes())
            .is_err()
    );
    assert_eq!(ingress.checkpoint().unwrap(), before);
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert!(
        ingress
            .guard_request(
                2,
                &tl::functions::messages::ReceivedQueue { max_qts: 40 }.to_bytes()
            )
            .is_err()
    );
}

async fn durable_raw_ingress_survives_absent_volatile_forwarding() {
    let (_dir, ingress, pool) = fixture(32).await;
    let (_tx, mut receiver) = receiver(&ingress, &pool).await;
    ingress
        .stage_live(
            2,
            &live(41, 2, b"owed before any volatile queue delivery".to_vec()),
        )
        .unwrap();
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        40
    );
    let output = tokio::time::timeout(Duration::from_secs(1), receiver.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.0.len(), 1);
    assert_eq!(ingress.pending_batches().unwrap().len(), 1);
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        41
    );
}

#[tokio::test]
async fn corrupt_declared_frontier_cannot_authorize_acknowledgment_after_batch_pruning() {
    failed_protocol_write_preserves_exact_receipt_for_reconstruction().await;
    let (dir, ingress, pool) = fixture(32).await;
    let mut protocol = protocol(&ingress);
    let (tx, mut receiver) = receiver(&ingress, &pool).await;
    let frame = frame();
    capture(&mut receiver, &tx, live(41, 2, frame.clone())).await;
    let id = ingress.pending_batches().unwrap()[0].id;
    ingress
        .apply(id, 0, async { protocol.receive(7, &frame, None, 124) })
        .await
        .unwrap()
        .unwrap();
    ingress.prune_completed().unwrap();
    assert!(ingress.pending_batches().unwrap().is_empty());
    let binding = ingress.binding().unwrap();
    {
        let mut core = ingress.core.lock().unwrap();
        let mut state = core.backend.load().unwrap().unwrap();
        let mut ledger: serde_json::Value =
            serde_json::from_slice(state.private_records[super::ledger::RECORD].as_bytes())
                .unwrap();
        let mut bad = MessageBoxes::new();
        bad.set_state(tl::types::updates::State {
            pts: 10,
            qts: 99,
            date: 124,
            seq: 2,
            unread_count: 0,
        });
        ledger["checkpoint"] = hex::encode(bad.durable_checkpoint().unwrap()).into();
        state.private_records.insert(
            super::ledger::RECORD.into(),
            crate::storage::SecretRecord::new(serde_json::to_vec(&ledger).unwrap()),
        );
        core.backend.commit(&state).unwrap();
    }
    drop(receiver);
    drop(protocol);
    drop(pool);
    drop(ingress);
    let restored = EncryptedIngressStorage::open(
        dir.path().join("state/custody.bin"),
        [9; 32],
        binding.clone(),
        1024 * 1024,
    )
    .unwrap();
    assert!(
        DurableIngress::new(
            restored,
            IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap()
        )
        .is_err(),
        "unbacked declared qts must not become an acknowledgment frontier"
    );
    let mut restored = EncryptedIngressStorage::open(
        dir.path().join("state/custody.bin"),
        [9; 32],
        binding,
        1024 * 1024,
    )
    .unwrap();
    let mut state = restored.load().unwrap().unwrap();
    assert!(!state.chats.is_empty());
    state.private_records.remove(super::ledger::RECORD);
    restored.commit(&state).unwrap();
    assert!(
        DurableIngress::new(
            restored,
            IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap()
        )
        .is_err(),
        "an existing protocol snapshot without its receipt journal must refuse a fresh baseline"
    );
}
