//! September audit regressions exercised through native protocol and file APIs.
#![allow(clippy::unwrap_used)]
#[path = "support_primitive/compile_boundary.rs"]
mod compile_boundary;
mod support_protocol;
use kiaro_secret_chat::{
    Serializable,
    chat::Outgoing,
    files::{FileKey, save_file},
    sequence,
};
use support_protocol::*;
#[test]
fn old_replay_ignores_newer_monotonic_echo() {
    let mut c = chat(true, 5);
    sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap();
    sequence::accept(&mut c, envelope(1, 3, 144, true)).unwrap();
    assert!(
        sequence::accept(&mut c, envelope(0, 0, 144, true))
            .unwrap()
            .ready
            .is_empty()
    );
    assert_ready(&c);
}
#[test]
fn queued_echo_commits_in_sequence_order() {
    let mut c = chat(true, 5);
    sequence::accept(&mut c, envelope(2, 3, 144, true)).unwrap();
    sequence::accept(&mut c, envelope(0, 1, 144, true)).unwrap();
    assert_eq!(
        sequence::accept(&mut c, envelope(1, 2, 144, true))
            .unwrap()
            .ready
            .len(),
        2
    );
    assert_eq!(c.peer_in_seq_no, 3);
}
#[test]
fn drained_gap_rechecks_predecessor_echo() {
    let mut c = chat(true, 5);
    sequence::accept(&mut c, envelope(2, 1, 144, true)).unwrap();
    sequence::accept(&mut c, envelope(0, 2, 144, true)).unwrap();
    assert!(sequence::accept(&mut c, envelope(1, 2, 144, true)).is_err());
}
#[test]
fn second_hole_gets_new_wire_span() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(1, 0, 144, true)).unwrap();
    sequence::accept(&mut c, envelope(3, 0, 144, true)).unwrap();
    assert_eq!(
        sequence::accept(&mut c, envelope(0, 0, 144, true))
            .unwrap()
            .resend,
        Some((4, 4))
    );
}
fn negative(raw: i32) {
    let mut c = chat(true, 0);
    let env = from_wrapper(wrap(1, raw * 2, 144));
    assert!(sequence::accept(&mut c, env).is_err());
    assert_eq!(c.in_seq_no, 0);
    assert!(c.gaps.is_empty());
}
#[test]
fn negative_outbound_minus_one() {
    negative(-1);
}
#[test]
fn negative_outbound_minus_two() {
    negative(-2);
}
fn wrong_end(end: i32) {
    let mut c = chat(true, 2);
    for seq in [1, 3] {
        c.outbox.insert(
            seq,
            Outgoing {
                seq_no: seq,
                random_id: i64::from(seq),
                frame: vec![1; 40],
                body: vec![],
                failures: 0,
                service: false,
                file: None,
                pending: false,
                obligation_generation: 1,
            },
        );
    }
    assert!(sequence::answer_resend(&c, 1, end).is_err());
}
#[test]
fn resend_end_even_two_refused() {
    wrong_end(2);
}
#[test]
fn resend_end_even_four_refused() {
    wrong_end(4);
}
#[test]
fn negative_file_size_is_rejected_at_native_type_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("existing.bin");
    std::fs::write(&target, b"ORIGINAL").unwrap();
    compile_boundary::refuses_program(
        "pub fn check(){let key=kiaro_secret_chat::files::FileKey::new(&[0;32],&[0;32]).unwrap();let mut input:&[u8]=&[];let _=kiaro_secret_chat::files::save_file(&mut input,std::path::Path::new(\"never-written.bin\"),-1,&key,0);}",
        "E0600",
    );
    assert_eq!(std::fs::read(target).unwrap(), b"ORIGINAL");
}
fn invalid_download(size: u64, length: usize) {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("existing.bin");
    std::fs::write(&target, b"ORIGINAL").unwrap();
    let key = FileKey::new(&[0; 32], &[0; 32]).unwrap();
    let bytes = vec![0; length];
    assert!(
        save_file(
            &mut bytes.as_slice(),
            &target,
            size,
            &key,
            key.fingerprint()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(target).unwrap(), b"ORIGINAL");
}
#[test]
fn truncated_download_cannot_replace_existing_file() {
    invalid_download(17, 16);
}
#[test]
fn unaligned_download_cannot_replace_existing_file() {
    invalid_download(1, 17);
}
#[test]
fn trailing_tl_bytes_are_refused() {
    let mut bytes = envelope(0, 0, 144, true).body;
    bytes.extend_from_slice(&[0; 4]);
    assert!(kiaro_secret_chat::framing::unwrap(&bytes).is_err());
}
#[test]
fn non_message_inner_constructor_is_refused() {
    let mut bytes = envelope(0, 0, 144, true).body;
    bytes.truncate(48);
    bytes.extend_from_slice(&kiaro_secret_chat::enums::DecryptedMessageAction::Noop.to_bytes());
    assert!(kiaro_secret_chat::framing::unwrap(&bytes).is_err());
}
