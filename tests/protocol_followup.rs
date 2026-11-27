use kiaro_secret_chat::ported::schema_secret_tl;
use kiaro_secret_chat::protocol::{
    ChatRole, ChatState, SecretChat, decrypt_frame, encrypt_frame, wrap_frame,
};
use serde_json::{Value, json};

const INITIAL_KEY: [u8; 256] = [7; 256];

fn ready_chat(role: ChatRole) -> SecretChat {
    let mut chat = SecretChat::new(7, 8, 9, "followup-audit", role);
    chat.adopt_key(&INITIAL_KEY).unwrap();
    chat
}

fn dh_prime() -> Vec<u8> {
    hex::decode(concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
        "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
        "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
        "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    ))
    .unwrap()
}

fn noop() -> Value {
    json!({"id":0xa82fdd63_u32,"fields":{}})
}

fn receive_action(chat: &mut SecretChat, frame: &[u8]) -> Value {
    let messages = chat.receive_encrypted_frames(frame).unwrap();
    assert_eq!(messages.len(), 1);
    messages[0].action.clone().unwrap()
}

fn accepted_pair() -> (SecretChat, SecretChat, Value) {
    let mut alice = ready_chat(ChatRole::Initiator);
    let mut bob = ready_chat(ChatRole::Acceptor);
    for chat in [&mut alice, &mut bob] {
        chat.configure_dh(2, &dh_prime()).unwrap();
    }
    let request = alice.start_rekey().unwrap().unwrap();
    let request = receive_action(&mut bob, &request.frame);
    let accept = bob.handle_rekey_action(&request).unwrap().remove(0);
    let accept = receive_action(&mut alice, &accept.frame);
    (alice, bob, accept)
}

fn committed_pair() -> (SecretChat, SecretChat, Vec<u8>) {
    let (mut alice, bob, accept) = accepted_pair();
    let commit = alice.handle_rekey_action(&accept).unwrap().remove(0);
    (alice, bob, commit.frame)
}

fn text_frame(sequence: i64, acknowledged: i64, layer: i64) -> Vec<u8> {
    let body = wrap_frame(
        &json!({"random_id":100 + sequence,"message":"previous shared key"}),
        layer,
        acknowledged * 2,
        sequence * 2 + 1,
    )
    .unwrap();
    encrypt_frame(&INITIAL_KEY, &body, 0).unwrap()
}

fn service_frame(sequence: i64, acknowledged: i64, action: Value) -> Vec<u8> {
    let body = schema_secret_tl::encode_object(json!({"id":0x1be31789_u32,"fields":{
        "random_bytes":{"hex":"07".repeat(31)},"layer":73,
        "in_seq_no":acknowledged * 2,"out_seq_no":sequence * 2 + 1,
        "message":{"id":0x73164160_u32,"fields":{"random_id":200 + sequence,"action":action}}
    }}))
    .unwrap();
    encrypt_frame(&INITIAL_KEY, &hex::decode(body).unwrap(), 0).unwrap()
}

#[test]
fn previous_shared_key_cannot_authorize_a_sequence_after_new_key_traffic() {
    let (mut alice, mut bob, _commit) = committed_pair();
    let next = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&next.frame)
            .unwrap()
            .is_empty()
    );
    assert!(bob.gap_requested());
    assert!(
        bob.receive_encrypted_frames(&text_frame(3, 1, 73)).is_err(),
        "a retained previous shared key must only recover sequences before the switch"
    );
}

#[test]
fn switching_keys_checks_the_key_of_already_buffered_future_frames() {
    let (mut alice, mut bob, _commit) = committed_pair();
    assert!(
        bob.receive_encrypted_frames(&text_frame(3, 1, 73))
            .unwrap()
            .is_empty()
    );
    let next = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&next.frame).is_err(),
        "a queued previous-key frame must not survive beyond authenticated new-key traffic"
    );
}

#[test]
fn commit_rejects_buffered_previous_key_frames_after_its_sequence() {
    let (_alice, mut bob, commit) = committed_pair();
    assert!(
        bob.receive_encrypted_frames(&text_frame(3, 1, 73))
            .unwrap()
            .is_empty()
    );
    assert!(
        bob.receive_encrypted_frames(&commit).is_err(),
        "a valid commitment must constrain queued ciphertext to the preceding key interval"
    );
}

#[test]
fn later_new_key_gap_does_not_extend_previous_key_retention() {
    let (mut alice, mut bob, commit) = committed_pair();
    let first_new = alice.prepare_service_action(&noop()).unwrap();
    let _missing_new = alice.prepare_service_action(&noop()).unwrap();
    let later_new = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&first_new.frame)
            .unwrap()
            .is_empty()
    );
    assert!(
        bob.receive_encrypted_frames(&later_new.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(bob.receive_encrypted_frames(&commit).unwrap().len(), 2);
    assert!(bob.gap_requested());
    assert_eq!(bob.in_seq_no(), 3);
    assert!(
        bob.start_rekey().unwrap().is_some(),
        "only a new-key gap remains, so the previous shared key must have been retired"
    );
}

#[test]
fn buffered_successor_acknowledgement_is_checked_before_immediate_resend() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    for _ in 0..2 {
        bob.prepare_service_action(&noop()).unwrap();
    }
    assert!(
        bob.receive_encrypted_frames(&text_frame(2, 1, 73))
            .unwrap()
            .is_empty()
    );
    let resend = service_frame(
        1,
        2,
        json!({"id":0x511110b0_u32,"fields":{
            "start_seq_no":0,"end_seq_no":2,
        }}),
    );
    assert!(
        bob.receive_encrypted_frames(&resend).is_err(),
        "known successor acknowledgement contradicts the Resend wrapper"
    );
    assert!(bob.take_immediate_actions().is_empty());
}

#[test]
fn authenticated_replay_is_dropped_before_parsing_its_inner_message() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    assert_eq!(
        bob.receive_encrypted_frames(&text_frame(0, 0, 73))
            .unwrap()
            .len(),
        1
    );
    let mut duplicate = wrap_frame(&json!({"message":"repeated"}), 73, 0, 1).unwrap();
    duplicate[48..52].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());
    let duplicate = encrypt_frame(&INITIAL_KEY, &duplicate, 0).unwrap();
    assert!(
        bob.receive_encrypted_frames(&duplicate)
            .is_ok_and(|messages| messages.is_empty()),
        "an already processed sequence must be dropped without inspecting its body"
    );
    assert_eq!(bob.in_seq_no(), 1);
    assert_eq!(
        bob.receive_encrypted_frames(&text_frame(1, 0, 73))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn delayed_commit_tightens_the_boundary_and_rechecks_buffered_key_provenance() {
    let (mut alice, mut bob, commit) = committed_pair();
    let _missing_second = alice.prepare_service_action(&noop()).unwrap();
    let _missing_third = alice.prepare_service_action(&noop()).unwrap();
    let fourth = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&text_frame(3, 1, 73))
            .unwrap()
            .is_empty()
    );
    assert!(
        bob.receive_encrypted_frames(&fourth.frame)
            .unwrap()
            .is_empty()
    );
    assert!(bob.receive_encrypted_frames(&commit).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
}

#[test]
fn delayed_commit_still_checks_its_fingerprint_after_implicit_key_adoption() {
    let (mut alice, mut bob, commit) = committed_pair();
    let next = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&next.frame)
            .unwrap()
            .is_empty()
    );
    let body = decrypt_frame(&INITIAL_KEY, &commit, 0).unwrap();
    let mut decoded = schema_secret_tl::decode_object(hex::encode(body), String::new(), 0).unwrap();
    let claimed = &mut decoded["object"]["fields"]["message"]["fields"]["action"]["fields"]["key_fingerprint"];
    *claimed = json!(claimed.as_i64().unwrap() ^ 1);
    let body = schema_secret_tl::encode_object(decoded["object"].clone()).unwrap();
    let invalid = encrypt_frame(&INITIAL_KEY, &hex::decode(body).unwrap(), 0).unwrap();
    assert!(bob.receive_encrypted_frames(&invalid).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
}

#[test]
fn valid_delayed_commit_retires_previous_key_before_unrelated_new_key_gaps_close() {
    let (mut alice, mut bob, commit) = committed_pair();
    let second = alice.prepare_service_action(&noop()).unwrap();
    let third = alice.prepare_service_action(&noop()).unwrap();
    let fourth = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&fourth.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(bob.receive_encrypted_frames(&commit).unwrap().len(), 1);
    assert_eq!(bob.in_seq_no(), 2);
    assert!(bob.gap_requested());
    // This fingerprint is no longer retained, so it cannot close the healthy chat.
    assert!(bob.receive_encrypted_frames(&text_frame(2, 1, 73)).is_err());
    assert_eq!(bob.state(), ChatState::Ready);
    assert!(
        bob.receive_encrypted_frames(&third.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        bob.receive_encrypted_frames(&second.frame).unwrap().len(),
        3
    );
    assert_eq!(bob.in_seq_no(), 5);
    assert!(!bob.gap_requested());
}

#[test]
fn previous_key_message_before_commit_remains_recoverable() {
    let (mut alice, mut bob, accept) = accepted_pair();
    let preceding = alice.prepare_service_action(&noop()).unwrap();
    let commit = alice.handle_rekey_action(&accept).unwrap().remove(0);
    let following = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&following.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        bob.receive_encrypted_frames(&preceding.frame)
            .unwrap()
            .len(),
        1
    );
    assert!(bob.gap_requested());
    assert_eq!(
        bob.receive_encrypted_frames(&commit.frame).unwrap().len(),
        2
    );
    assert_eq!(bob.in_seq_no(), 4);
    assert_eq!(bob.state(), ChatState::Ready);
}

#[test]
fn initiator_waits_for_peer_confirmation_and_only_the_preceding_gap() {
    let (mut alice, mut bob, commit) = committed_pair();
    let preceding = bob.prepare_service_action(&noop()).unwrap();
    let commit = receive_action(&mut bob, &commit);
    let confirmation = bob.handle_rekey_action(&commit).unwrap().remove(0);
    let _missing = bob.prepare_service_action(&noop()).unwrap();
    let later = bob.prepare_service_action(&noop()).unwrap();
    assert!(alice.start_rekey().unwrap().is_none());
    assert!(
        alice
            .receive_encrypted_frames(&confirmation.frame)
            .unwrap()
            .is_empty()
    );
    assert!(alice.start_rekey().unwrap().is_none());
    assert!(
        alice
            .receive_encrypted_frames(&later.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        alice
            .receive_encrypted_frames(&preceding.frame)
            .unwrap()
            .len(),
        2
    );
    assert!(alice.gap_requested());
    assert!(alice.start_rekey().unwrap().is_some());
}

#[test]
fn conflicting_keys_for_one_pending_sequence_close_without_adopting() {
    let (mut alice, mut bob, _commit) = committed_pair();
    assert!(
        bob.receive_encrypted_frames(&text_frame(2, 1, 73))
            .unwrap()
            .is_empty()
    );
    let duplicate = alice.prepare_service_action(&noop()).unwrap();
    assert!(bob.receive_encrypted_frames(&duplicate.frame).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
    assert!(!bob.take_pending_noop());
}

#[test]
fn pending_key_replay_of_an_already_processed_sequence_does_not_adopt() {
    let (mut alice, mut bob, _commit) = committed_pair();
    bob.receive_encrypted_frames(&text_frame(1, 1, 73)).unwrap();
    bob.receive_encrypted_frames(&text_frame(2, 1, 73)).unwrap();
    let before = bob.key_lifecycle();
    let duplicate = alice.prepare_service_action(&noop()).unwrap();
    assert!(
        bob.receive_encrypted_frames(&duplicate.frame)
            .unwrap()
            .is_empty()
    );
    assert_eq!(bob.key_lifecycle(), before);
    assert!(!bob.take_pending_noop());
}

#[test]
fn queued_replay_is_dropped_without_replacing_the_original_body() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    assert!(
        bob.receive_encrypted_frames(&text_frame(1, 0, 73))
            .unwrap()
            .is_empty()
    );
    let mut duplicate = wrap_frame(&json!({"message":"repeated"}), 73, 0, 3).unwrap();
    duplicate[48..52].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());
    let duplicate = encrypt_frame(&INITIAL_KEY, &duplicate, 0).unwrap();
    assert!(bob.receive_encrypted_frames(&duplicate).unwrap().is_empty());
    let accepted = bob.receive_encrypted_frames(&text_frame(0, 0, 73)).unwrap();
    assert_eq!(accepted.len(), 2);
    assert_eq!(accepted[1].text.as_deref(), Some("previous shared key"));
}

#[test]
fn duplicate_header_still_requires_nonnegative_counters_and_correct_parity() {
    for (input, output) in [(1, 1), (0, -1), (-2, 1), (0, 0)] {
        let mut bob = ready_chat(ChatRole::Acceptor);
        bob.receive_encrypted_frames(&text_frame(0, 0, 73)).unwrap();
        let body = wrap_frame(&json!({"message":"repeated"}), 73, input, output).unwrap();
        let duplicate = encrypt_frame(&INITIAL_KEY, &body, 0).unwrap();
        assert!(bob.receive_encrypted_frames(&duplicate).is_err());
        assert_eq!(bob.state(), ChatState::Closed);
    }
}

#[test]
fn duplicate_detection_never_bypasses_frame_authentication() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    let mut duplicate = text_frame(0, 0, 73);
    bob.receive_encrypted_frames(&duplicate).unwrap();
    duplicate[8] ^= 1;
    assert!(bob.receive_encrypted_frames(&duplicate).is_err());
    assert_eq!(bob.state(), ChatState::Ready);
    assert_eq!(bob.in_seq_no(), 1);
}

#[test]
fn buffered_predecessor_acknowledgement_cannot_regress() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    for _ in 0..2 {
        bob.prepare_service_action(&noop()).unwrap();
    }
    assert!(
        bob.receive_encrypted_frames(&text_frame(1, 2, 73))
            .unwrap()
            .is_empty()
    );
    assert!(bob.receive_encrypted_frames(&text_frame(2, 1, 73)).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
}

#[test]
fn acknowledgement_order_follows_sequence_positions_instead_of_arrival_order() {
    let mut bob = ready_chat(ChatRole::Acceptor);
    for _ in 0..2 {
        bob.prepare_service_action(&noop()).unwrap();
    }
    assert!(
        bob.receive_encrypted_frames(&text_frame(2, 2, 144))
            .unwrap()
            .is_empty()
    );
    assert!(
        bob.receive_encrypted_frames(&text_frame(1, 1, 73))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        bob.receive_encrypted_frames(&text_frame(0, 0, 73))
            .unwrap()
            .len(),
        3
    );
    assert_eq!(bob.peer_in_seq_no(), 2);
    assert_eq!(bob.wrapper_layer(), 144);
}

#[test]
fn buffered_wrapper_layer_neighbors_are_checked_in_both_directions() {
    for ((first, first_layer), (second, second_layer)) in [((1, 144), (2, 73)), ((2, 73), (1, 144))]
    {
        let mut bob = ready_chat(ChatRole::Acceptor);
        assert!(
            bob.receive_encrypted_frames(&text_frame(first, 0, first_layer))
                .unwrap()
                .is_empty()
        );
        assert!(
            bob.receive_encrypted_frames(&text_frame(second, 0, second_layer))
                .is_err()
        );
        assert_eq!(bob.state(), ChatState::Closed);
    }
}

#[test]
fn invalid_wrapper_header_does_not_turn_ignored_bytes_into_a_fatal_counter_error() {
    for (random_length, layer) in [(14, 73), (31, 45)] {
        let mut bob = ready_chat(ChatRole::Acceptor);
        let body = schema_secret_tl::encode_object(json!({"id":0x1be31789_u32,"fields":{
            "random_bytes":{"hex":"07".repeat(random_length)},"layer":layer,
            "in_seq_no":1,"out_seq_no":1,
            "message":{"id":0x73164160_u32,"fields":{"random_id":200,"action":noop()}}
        }}))
        .unwrap();
        let invalid = encrypt_frame(&INITIAL_KEY, &hex::decode(body).unwrap(), 0).unwrap();
        assert!(bob.receive_encrypted_frames(&invalid).is_err());
        assert_eq!(bob.state(), ChatState::Ready);
        assert_eq!(bob.in_seq_no(), 0);
    }
}

#[test]
fn matching_commit_cannot_use_the_pending_or_already_adopted_new_key() {
    for adopt_pending_first in [false, true] {
        let (mut alice, mut bob, commit) = committed_pair();
        let body = decrypt_frame(&INITIAL_KEY, &commit, 0).unwrap();
        let decoded = schema_secret_tl::decode_object(hex::encode(body), String::new(), 0).unwrap();
        let action = &decoded["object"]["fields"]["message"]["fields"]["action"];
        if adopt_pending_first {
            let first_new = alice.prepare_service_action(&noop()).unwrap();
            assert!(
                bob.receive_encrypted_frames(&first_new.frame)
                    .unwrap()
                    .is_empty()
            );
        }
        let wrong_key = alice.prepare_service_action(action).unwrap();
        assert!(bob.receive_encrypted_frames(&wrong_key.frame).is_err());
        assert_eq!(bob.state(), ChatState::Closed);
    }
}
