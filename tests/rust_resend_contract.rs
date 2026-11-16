//! Exact-byte replay retention and immediately interpreted resend requests.
#![allow(clippy::unwrap_used)]
#[path = "support_protocol/foundation.rs"]
mod foundation;
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
mod support_protocol;
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, Outgoing},
    error::Error,
    manager::{Event, Options, RpcFailure},
    protocol::Protocol,
    sequence,
    storage::{FileStorage, State, Storage},
    types,
};
use std::sync::{Arc, Mutex};
use support_protocol::*;
fn retained(count: i32) -> Chat {
    let mut c = chat(true, count);
    for raw in 0..count {
        let seq = 2 * raw + 1;
        c.outbox.insert(
            seq,
            Outgoing {
                seq_no: seq,
                random_id: i64::from(raw),
                frame: vec![1; 40],
                body: format!("body-{raw}").into_bytes(),
                failures: 0,
                service: false,
                file: None,
                pending: false,
                obligation_generation: 1,
            },
        );
    }
    c
}
fn bodies(c: &Chat, start: i32, end: i32) -> Vec<Vec<u8>> {
    sequence::answer_resend(c, start, end)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}
#[test]
fn requested_span_is_returned_in_wire_order() {
    assert_eq!(
        bodies(&retained(6), 3, 7),
        vec![b"body-1".to_vec(), b"body-2".to_vec(), b"body-3".to_vec()]
    );
}
#[test]
fn span_bounds_are_inclusive() {
    let c = retained(4);
    assert_eq!(bodies(&c, 1, 1), vec![b"body-0".to_vec()]);
    assert_eq!(bodies(&c, 1, 7).len(), 4);
}
#[test]
fn a_single_message_span_is_satisfied() {
    assert_eq!(bodies(&retained(3), 5, 5), vec![b"body-2".to_vec()]);
}
#[test]
fn repeat_resend_does_not_consume_retention() {
    let c = retained(3);
    let first = bodies(&c, 1, 5);
    assert_eq!(first, bodies(&c, 1, 5));
    assert_eq!(c.outbox.len(), 3);
    let mut p = protocol();
    let original = p.send_message(7, "in flight".into(), None, None).unwrap();
    let replacement = p
        .receive(7, &resend(0, 1, 1), None, 1)
        .unwrap()
        .sends
        .remove(0);
    assert_eq!(replacement.outgoing.frame, original.outgoing.frame);
    p.confirm_sent(&original).unwrap();
    assert!(
        p.chat_record(7).unwrap().outbox[&1].pending,
        "a late original send cannot clear the newly committed retransmission"
    );
    assert_eq!(p.retry_pending(7).unwrap().len(), 1);
    p.confirm_sent(&replacement).unwrap();
    assert!(p.retry_pending(7).unwrap().is_empty());
}
fn refused(c: &Chat, start: i32, end: i32) {
    let error = sequence::answer_resend(c, start, end).err().unwrap();
    assert!(matches!(error, Error::ResendUnavailable));
}
#[test]
fn span_below_retention_refuses() {
    let mut c = retained(3);
    c.outbox.remove(&1);
    refused(&c, 1, 5);
}
#[test]
fn span_beyond_sent_refuses() {
    refused(&retained(2), 1, 99);
}
#[test]
fn span_wider_than_reference_cap_refuses() {
    assert_eq!(sequence::MAX_RESEND_COUNT, 1000);
    refused(&retained(1001), 1, 2001);
}
#[test]
fn exact_reference_cap_is_satisfied() {
    assert_eq!(bodies(&retained(1000), 1, 1999).len(), 1000);
}
#[test]
fn inverted_span_refuses() {
    refused(&retained(5), 7, 3);
}
#[test]
fn refusal_never_formats_retained_plaintext() {
    let mut c = retained(2);
    c.outbox.get_mut(&1).unwrap().body = b"PRIVATE-RETAINED-SENTENCE".to_vec();
    let error = sequence::answer_resend(&c, 1, 4001).err().unwrap();
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains("PRIVATE-RETAINED-SENTENCE"));
    assert!(rendered.len() < 256);
}
#[test]
fn peer_acknowledgment_prunes_only_earlier_wire_rows() {
    let mut p = protocol();
    for n in 0..5 {
        p.send_message(7, n.to_string(), None, None).unwrap();
    }
    p.receive(7, &frame(&envelope(0, 2, 144, true).body), None, 1)
        .unwrap();
    let rows = p
        .chat_record(7)
        .unwrap()
        .outbox
        .into_keys()
        .collect::<Vec<_>>();
    assert_eq!(rows, vec![5, 7, 9]);
}
#[test]
fn exact_retention_survives_file_backend_reconstruction() {
    foundation::pending_send_retains_exact_identity_and_ciphertext();
    foundation::confirmed_send_remains_available_for_peer_resend();
    foundation::two_sides_exchange_and_acknowledge_delivery();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = FileStorage::open(&path).unwrap();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    store.commit(&state).unwrap();
    let mut p = Protocol::open(store, "11".repeat(32)).unwrap();
    p.start().unwrap();
    let mut original = Vec::new();
    for text in ["one", "two", "three"] {
        original.push(
            p.send_message(7, text.into(), None, None)
                .unwrap()
                .outgoing
                .frame,
        );
    }
    p.stop().unwrap();
    drop(p);
    let p = Protocol::open(FileStorage::open(&path).unwrap(), "11".repeat(32)).unwrap();
    let c = p.chat_record(7).unwrap();
    let answer = sequence::answer_resend(&c, 1, 5).unwrap();
    assert_eq!(
        answer.iter().map(|r| r.frame.clone()).collect::<Vec<_>>(),
        original
    );
    assert_eq!(
        answer
            .iter()
            .map(|r| r.random_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}
fn resend(raw: i32, start: i32, end: i32) -> Vec<u8> {
    frame(
        &kiaro_secret_chat::framing::wrap(
            types::DecryptedMessageService {
                random_id: 100 + i64::from(raw),
                action: types::DecryptedMessageActionResend {
                    start_seq_no: start,
                    end_seq_no: end,
                }
                .into(),
            }
            .into(),
            144,
            1,
            2 * raw,
        )
        .unwrap()
        .to_bytes(),
    )
}
#[tokio::test]
async fn out_of_order_request_is_answered_once_before_gap_closes() {
    foundation::test_a_resend_request_is_answered_even_while_it_is_out_of_order();
    foundation::resend_precedes_later_ready_acknowledgement_pruning();

    let (a, store, w, id) = seeded::ready(Options::default()).await;
    a.send_message(id, "retained", None, None).await.unwrap();
    let original = w.lock().unwrap().sent.last().unwrap().outgoing.clone();
    let before = w.lock().unwrap().sent.len();
    let request = resend(2, 1, 1);
    a.receive(id, &request, None, 1).await.unwrap();
    assert_eq!(
        w.lock().unwrap().sent[before..]
            .iter()
            .filter(|s| s.outgoing.frame == original.frame)
            .count(),
        1
    );
    a.receive(id, &request, None, 1).await.unwrap();
    for raw in [0, 1] {
        a.receive(id, &frame(&envelope(raw, 0, 144, true).body), None, 1)
            .await
            .unwrap();
    }
    assert_eq!(
        w.lock().unwrap().sent[before..]
            .iter()
            .filter(|s| s.outgoing.frame == original.frame)
            .count(),
        1
    );
    assert!(store.load().unwrap().unwrap().chats[&id].gaps.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn lost_resend_answer_retries_original_frames_without_new_sequences() {
    foundation::test_a_resend_answer_lost_on_the_network_is_sent_again();

    let (a, store, w, id) = seeded::ready(Options::default()).await;
    for text in ["one", "two", "three"] {
        a.send_message(id, text, None, None).await.unwrap();
    }
    let original = w
        .lock()
        .unwrap()
        .sent
        .iter()
        .map(|s| s.outgoing.frame.clone())
        .collect::<Vec<_>>();
    let seen = Arc::new(Mutex::new(0));
    let observed = seen.clone();
    a.on("ServiceActionReceived", "capture", move |event| {
        if matches!(event, Event::ServiceActionReceived { .. }) {
            *observed.lock().unwrap() += 1;
        }
        Ok(())
    })
    .unwrap();
    w.lock()
        .unwrap()
        .fail_sends_from
        .insert(1000, RpcFailure::Transient);
    assert!(matches!(
        a.receive(id, &resend(0, 1, 5), None, 1).await,
        Err(Error::SendPending { .. })
    ));
    assert_eq!(*seen.lock().unwrap(), 1);
    a.stop().await.unwrap();
    a.start().await.unwrap();
    w.lock().unwrap().fail_sends_from.remove(&1000);
    let before = w.lock().unwrap().sent.len();
    a.retry_pending(id).await.unwrap();
    assert_eq!(
        w.lock().unwrap().sent[before..]
            .iter()
            .map(|s| s.outgoing.frame.clone())
            .collect::<Vec<_>>(),
        original
    );
    assert_eq!(store.load().unwrap().unwrap().chats[&id].out_seq_no, 3);
    a.stop().await.unwrap();
}
