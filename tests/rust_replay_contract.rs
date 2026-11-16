#![allow(clippy::unwrap_used)]
#[path = "support_protocol/foundation.rs"]
mod foundation;
mod support_protocol;
use kiaro_secret_chat::sequence;
use support_protocol::*;
#[test]
fn expected_message_is_delivered() {
    let mut c = chat(true, 0);
    assert_eq!(
        ids(sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap()),
        [0]
    );
    assert_eq!(c.in_seq_no, 1);
}
#[test]
fn ordered_run_is_delivered() {
    let mut c = chat(true, 0);
    let mut got = Vec::new();
    for raw in 0..5 {
        got.extend(ids(
            sequence::accept(&mut c, envelope(raw, 0, 144, true)).unwrap()
        ));
    }
    assert_eq!(got, [0, 1, 2, 3, 4]);
    assert_eq!(c.in_seq_no, 5);
}
#[test]
fn repeated_message_not_redelivered() {
    foundation::replay_is_not_redelivered();

    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap();
    assert!(
        sequence::accept(&mut c, envelope(0, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(c.in_seq_no, 1);
    assert_ready(&c);
}
#[test]
fn old_message_below_counter_is_dropped() {
    let mut c = chat(true, 0);
    for raw in 0..4 {
        sequence::accept(&mut c, envelope(raw, 0, 144, true)).unwrap();
    }
    assert!(
        sequence::accept(&mut c, envelope(1, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(c.in_seq_no, 4);
}
#[test]
fn old_replay_inner_content_is_not_inspected() {
    let mut c = chat(true, 0);
    let original = envelope(0, 0, 144, true);
    sequence::accept(&mut c, original.clone()).unwrap();
    let mut malformed = original;
    malformed.body[48..52].copy_from_slice(&0xdeadbeefu32.to_le_bytes());
    assert!(
        sequence::accept(&mut c, malformed.clone())
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(c.in_seq_no, 1);
    let mut p = protocol();
    p.receive(7, &frame(&envelope(0, 0, 144, true).body), None, 1)
        .unwrap();
    assert!(p.receive(7, &frame(&malformed.body), None, 1).is_ok());
    assert_eq!(p.pending_deliveries(7).unwrap(), [0]);
}
#[test]
fn ahead_message_is_held_without_delivery() {
    let mut c = chat(true, 0);
    assert!(
        sequence::accept(&mut c, envelope(2, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(c.in_seq_no, 0);
    assert!(c.gaps.contains_key(&2));
}
#[test]
fn closing_hole_releases_all_in_order() {
    foundation::gap_requests_once_and_closes_in_order();
    foundation::sequence_mirroring_is_refused();
    foundation::excessive_gap_is_refused();
    foundation::unsent_peer_echo_is_refused();
    foundation::backwards_wrapper_layer_is_refused();

    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(2, 0, 144, true)).unwrap();
    sequence::accept(&mut c, envelope(1, 0, 144, true)).unwrap();
    assert_eq!(
        ids(sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap()),
        [0, 1, 2]
    );
    assert_eq!(c.in_seq_no, 3);
}
#[test]
fn partial_hole_stays_held() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(3, 0, 144, true)).unwrap();
    assert!(
        sequence::accept(&mut c, envelope(1, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(c.in_seq_no, 0);
}
#[test]
fn gap_requests_exact_missing_wire_span() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap();
    assert_eq!(
        sequence::accept(&mut c, envelope(4, 0, 144, true))
            .unwrap()
            .resend,
        Some((2, 6))
    );
}
#[test]
fn one_resend_request_per_hole() {
    let mut c = chat(true, 0);
    assert!(
        sequence::accept(&mut c, envelope(3, 0, 144, true))
            .unwrap()
            .resend
            .is_some()
    );
    for raw in [4, 5] {
        assert!(
            sequence::accept(&mut c, envelope(raw, 0, 144, true))
                .unwrap()
                .resend
                .is_none()
        );
    }
}
#[test]
fn closed_hole_allows_later_resend() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(1, 0, 144, true)).unwrap();
    assert!(
        !sequence::accept(&mut c, envelope(0, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert!(
        sequence::accept(&mut c, envelope(5, 0, 144, true))
            .unwrap()
            .resend
            .is_some()
    );
}
#[test]
fn nondecreasing_echo_is_accepted() {
    let mut c = chat(true, 5);
    for (raw, echo) in [(0, 1), (1, 1), (2, 3)] {
        sequence::accept(&mut c, envelope(raw, echo, 144, true)).unwrap();
    }
    assert_eq!(c.in_seq_no, 3);
    assert_eq!(c.peer_in_seq_no, 3);
}
#[test]
fn backwards_echo_is_refused() {
    let mut c = chat(true, 5);
    sequence::accept(&mut c, envelope(0, 3, 144, true)).unwrap();
    assert!(sequence::accept(&mut c, envelope(1, 1, 144, true)).is_err());
    assert_eq!(c.in_seq_no, 1);
}
#[test]
fn echo_beyond_sent_is_refused() {
    let mut c = chat(true, 2);
    sequence::accept(&mut c, envelope(0, 2, 144, true)).unwrap();
    assert!(sequence::accept(&mut c, envelope(1, 3, 144, true)).is_err());
    assert_eq!(c.in_seq_no, 1);
}
#[test]
fn echo_is_checked_before_gap_queue() {
    let mut c = chat(true, 1);
    assert!(sequence::accept(&mut c, envelope(4, 9, 144, true)).is_err());
    assert!(c.gaps.is_empty());
}
