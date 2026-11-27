use aes::{
    Aes256,
    cipher::{Array, BlockCipherEncrypt, KeyInit},
};
use kiaro_secret_chat::ported::schema_secret_tl;
use kiaro_secret_chat::protocol::{
    ChatRole, ChatState, SecretChat, compute_msg_key, decrypt_frame, derive_keys, encrypt_frame,
    key_fingerprint, wrap_frame,
};
use serde_json::json;

fn chat(key: &[u8; 256]) -> SecretChat {
    let mut chat = SecretChat::new(7, 1, 2, "audit", ChatRole::Acceptor);
    chat.adopt_key(key).unwrap();
    chat
}

fn frame(key: &[u8; 256], sequence: i64, layer: i64) -> Vec<u8> {
    let body = wrap_frame(
        &json!({"random_id": sequence + 1, "ttl": 0, "message": sequence.to_string()}),
        layer,
        0,
        sequence * 2 + 1,
    )
    .unwrap();
    encrypt_frame(key, &body, 0).unwrap()
}

#[test]
fn authenticated_gap_is_retained_until_its_predecessor_arrives() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    assert!(
        receiver
            .receive_encrypted_frame(&frame(&key, 1, 144))
            .is_err()
    );
    assert_eq!(receiver.in_seq_no(), 0);
    receiver
        .receive_encrypted_frame(&frame(&key, 0, 73))
        .unwrap();
    assert_eq!(
        receiver.in_seq_no(),
        2,
        "the buffered successor must be recovered"
    );
}

#[test]
fn closed_chat_cannot_be_resurrected_by_adopting_a_key() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    receiver.close();
    assert!(receiver.adopt_key(&key).is_err());
    assert_eq!(receiver.state(), ChatState::Closed);
}

#[test]
fn encryption_refuses_truncated_shared_keys() {
    for length in [0, 1, 255, 257] {
        assert!(
            encrypt_frame(&vec![0; length], &[0; 4], 0).is_err(),
            "length {length}"
        );
    }
}

#[test]
fn message_key_derivation_refuses_truncated_shared_keys() {
    for length in [0, 1, 255, 257] {
        assert!(
            compute_msg_key(&vec![0; length], &[0; 16], 0).is_err(),
            "length {length}"
        );
    }
}

#[test]
fn aes_key_derivation_refuses_wrong_message_key_lengths() {
    for length in [0, 1, 15, 17] {
        assert!(
            derive_keys(&[7; 256], &vec![0; length], 0).is_err(),
            "length {length}"
        );
    }
}

#[test]
fn tl_reserved_string_length_prefix_is_rejected() {
    let mut bytes = 0xb71e767a_u32.to_le_bytes().to_vec();
    bytes.push(255);
    bytes.extend([b'a'; 255]);
    assert!(schema_secret_tl::decode_object(hex::encode(bytes), String::new(), 0).is_err());
}

#[test]
fn tl_nonzero_string_padding_is_rejected() {
    assert!(schema_secret_tl::decode_object("7a761eb70161ff00".into(), String::new(), 0).is_err());
}

#[test]
fn tl_encoder_rejects_a_constructor_from_the_wrong_result_family() {
    let wrong = json!({"id":0x73164160_u32,"fields":{
        "random_id":1,
        "action":{"id":0x089f5c4a_u32,"fields":{}}
    }});
    assert!(schema_secret_tl::encode_object(wrong).is_err());
}

#[test]
fn wrapper_preserves_silent_and_no_webpage_flags() {
    let body = wrap_frame(
        &json!({"random_id":1,"message":"flags","silent":true,"no_webpage":true}),
        73,
        0,
        1,
    )
    .unwrap();
    let decoded = schema_secret_tl::decode_object(hex::encode(body), String::new(), 0).unwrap();
    let fields = &decoded["object"]["fields"]["message"]["fields"];
    assert_eq!(fields["silent"], true);
    assert_eq!(fields["no_webpage"], true);
}

fn authenticated_payload(key: &[u8; 256], body: &[u8], padding: usize) -> Vec<u8> {
    let mut payload = (body.len() as u32).to_le_bytes().to_vec();
    payload.extend_from_slice(body);
    payload.resize(payload.len() + padding, 0x5a);
    assert_eq!(payload.len() % 16, 0);
    let message_key = compute_msg_key(key, &payload, 0).unwrap();
    let (aes_key, aes_iv) = derive_keys(key, &message_key, 0).unwrap();
    let cipher = Aes256::new(&Array::from(aes_key));
    let mut previous_cipher: [u8; 16] = aes_iv[..16].try_into().unwrap();
    let mut previous_plain: [u8; 16] = aes_iv[16..].try_into().unwrap();
    for chunk in payload.as_chunks_mut::<16>().0 {
        let plain = *chunk;
        let mut block = Array::from(std::array::from_fn(|i| plain[i] ^ previous_cipher[i]));
        cipher.encrypt_block(&mut block);
        for i in 0..16 {
            chunk[i] = block[i] ^ previous_plain[i];
        }
        previous_plain = plain;
        previous_cipher.copy_from_slice(chunk);
    }
    let mut result = key_fingerprint(key).unwrap().to_le_bytes().to_vec();
    result.extend(message_key);
    result.extend(payload);
    result
}

#[test]
fn crypto_rejects_authenticated_padding_above_1024_bytes() {
    let key = [7; 256];
    let valid = authenticated_payload(&key, &[0; 12], 1024);
    assert_eq!(decrypt_frame(&key, &valid, 0).unwrap(), [0; 12]);
    let invalid = authenticated_payload(&key, &[], 1036);
    assert!(decrypt_frame(&key, &invalid, 0).is_err());
}

#[test]
fn crypto_rejects_authenticated_body_length_not_divisible_by_four() {
    let key = [7; 256];
    let invalid = authenticated_payload(&key, &[0; 3], 25);
    assert!(decrypt_frame(&key, &invalid, 0).is_err());
}

fn service_frame(key: &[u8; 256], sequence: i64, action: serde_json::Value) -> Vec<u8> {
    let body = schema_secret_tl::encode_object(json!({"id":0x1be31789_u32,"fields":{
        "random_bytes":{"hex":"07".repeat(31)}, "layer":73,
        "in_seq_no":0,"out_seq_no":sequence * 2 + 1,
        "message":{"id":0x73164160_u32,"fields":{"random_id":100 + sequence,"action":action}}
    }}))
    .unwrap();
    encrypt_frame(key, &hex::decode(body).unwrap(), 0).unwrap()
}

#[test]
fn notify_layer_raises_capability_without_rejecting_older_in_order_wrappers() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let notify = service_frame(&key, 1, json!({"id":0xf3048883_u32,"fields":{"layer":144}}));
    assert!(
        receiver
            .receive_encrypted_frames(&notify)
            .unwrap()
            .is_empty()
    );
    assert_eq!(receiver.negotiated_layer(), 144);
    assert_eq!(receiver.wrapper_layer(), 46);
    let recovered = receiver
        .receive_encrypted_frames(&frame(&key, 0, 73))
        .unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(receiver.wrapper_layer(), 73);
}

#[test]
fn resend_actions_bypass_a_gap_once_and_are_not_replayed_when_drained() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let action = json!({"id":0x511110b0_u32,"fields":{"start_seq_no":0,"end_seq_no":2}});
    let request = service_frame(&key, 1, action.clone());
    assert!(
        receiver
            .receive_encrypted_frames(&request)
            .unwrap()
            .is_empty()
    );
    assert_eq!(receiver.take_immediate_actions(), [action]);
    assert!(
        receiver
            .receive_encrypted_frames(&request)
            .unwrap()
            .is_empty()
    );
    assert!(receiver.take_immediate_actions().is_empty());
    let recovered = receiver
        .receive_encrypted_frames(&frame(&key, 0, 73))
        .unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[1].action.as_ref().unwrap()["id"], 0xa82fdd63_u32);
    assert!(receiver.take_immediate_actions().is_empty());
}

#[test]
fn authenticated_wrong_sequence_parity_closes_the_chat() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let body = wrap_frame(&json!({"message":"invalid"}), 73, 1, 1).unwrap();
    let invalid = encrypt_frame(&key, &body, 0).unwrap();
    assert!(receiver.receive_encrypted_frames(&invalid).is_err());
    assert_eq!(receiver.state(), ChatState::Closed);
}

#[test]
fn authenticated_unparseable_inner_message_closes_the_chat() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let mut body = wrap_frame(&json!({"message":"invalid"}), 73, 0, 1).unwrap();
    body[48..52].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());
    let invalid = encrypt_frame(&key, &body, 0).unwrap();
    assert!(receiver.receive_encrypted_frames(&invalid).is_err());
    assert_eq!(receiver.state(), ChatState::Closed);
}

#[test]
fn tl_encoder_has_the_same_nesting_bound_as_the_decoder() {
    let mut object = json!({"id":0x3f6d7b68_u32,"fields":{}});
    for _ in 0..32 {
        object = json!({"id":0xf7444763_u32,"fields":{"value":[object]}});
    }
    assert!(schema_secret_tl::encode_object(object).is_err());
}

#[test]
fn tl_caller_cannot_bypass_nesting_checks_with_negative_initial_depth() {
    assert!(schema_secret_tl::decode_object("687b6d3f".into(), String::new(), -1).is_err());
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

fn rekey_pair() -> (SecretChat, SecretChat) {
    let mut alice = SecretChat::new(7, 1, 2, "alice", ChatRole::Initiator);
    let mut bob = SecretChat::new(7, 1, 2, "bob", ChatRole::Acceptor);
    for side in [&mut alice, &mut bob] {
        side.adopt_key(&[7; 256]).unwrap();
        side.configure_dh(2, &dh_prime()).unwrap();
    }
    (alice, bob)
}

fn receive_action(chat: &mut SecretChat, wire: &[u8]) -> serde_json::Value {
    let messages = chat.receive_encrypted_frames(wire).unwrap();
    assert_eq!(messages.len(), 1);
    messages[0].action.clone().unwrap()
}

fn fingerprint(frame: &[u8]) -> i64 {
    i64::from_le_bytes(frame[..8].try_into().unwrap())
}

#[test]
fn runtime_rekey_exchanges_four_messages_and_preserves_sequence_counters() {
    let (mut alice, mut bob) = rekey_pair();
    let old = key_fingerprint(&[7; 256]).unwrap();
    let request = alice.start_rekey().unwrap().unwrap();
    assert_eq!(fingerprint(&request.frame), old);
    let action = receive_action(&mut bob, &request.frame);
    let accept = bob.handle_rekey_action(&action).unwrap().remove(0);
    assert_eq!(fingerprint(&accept.frame), old);
    let action = receive_action(&mut alice, &accept.frame);
    let commit = alice.handle_rekey_action(&action).unwrap().remove(0);
    assert_eq!(fingerprint(&commit.frame), old);
    let action = receive_action(&mut bob, &commit.frame);
    let noop = bob.handle_rekey_action(&action).unwrap().remove(0);
    assert_ne!(fingerprint(&noop.frame), old);
    let action = receive_action(&mut alice, &noop.frame);
    assert_eq!(action["id"], 0xa82fdd63_u32);
    assert!(alice.handle_rekey_action(&action).unwrap().is_empty());
    assert_eq!((alice.in_seq_no(), alice.out_seq_no()), (2, 2));
    assert_eq!((bob.in_seq_no(), bob.out_seq_no()), (2, 2));
    assert_eq!(alice.key_lifecycle(), bob.key_lifecycle());
    let next = alice
        .prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))
        .unwrap();
    assert_eq!(fingerprint(&next.frame), fingerprint(&noop.frame));
    assert_eq!(receive_action(&mut bob, &next.frame)["id"], 0xa82fdd63_u32);
    assert!(
        alice.start_rekey().unwrap().is_some(),
        "confirmation retires the previous key"
    );
}

#[test]
fn runtime_rekey_accepts_new_key_before_a_lost_commit_and_later_recovers_the_old_frame() {
    let (mut alice, mut bob) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = receive_action(&mut bob, &request.frame);
    let accept = bob.handle_rekey_action(&action).unwrap().remove(0);
    let action = receive_action(&mut alice, &accept.frame);
    let commit = alice.handle_rekey_action(&action).unwrap().remove(0);
    let next = alice
        .prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))
        .unwrap();
    assert!(
        bob.receive_encrypted_frames(&next.frame)
            .unwrap()
            .is_empty()
    );
    assert!(bob.gap_requested());
    assert!(bob.take_pending_noop());
    assert!(!bob.take_pending_noop());
    let recovered = bob.receive_encrypted_frames(&commit.frame).unwrap();
    assert_eq!(recovered.len(), 2);
    assert!(!bob.gap_requested());
    assert_eq!(bob.key_lifecycle(), alice.key_lifecycle());
    assert!(bob.start_rekey().unwrap().is_some());
}

#[test]
fn runtime_rekey_bad_commit_fingerprint_closes_the_accepting_chat() {
    let (mut alice, mut bob) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = receive_action(&mut bob, &request.frame);
    let accept = bob.handle_rekey_action(&action).unwrap().remove(0);
    let accepted = receive_action(&mut alice, &accept.frame);
    let invalid = json!({"id":0xec2e0b9b_u32,"fields":{
        "exchange_id":accepted["fields"]["exchange_id"],
        "key_fingerprint":accepted["fields"]["key_fingerprint"].as_i64().unwrap() ^ 1,
    }});
    assert!(bob.handle_rekey_action(&invalid).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
}

#[test]
fn runtime_rekey_without_stored_dh_parameters_closes_instead_of_silently_continuing() {
    let mut receiver = chat(&[7; 256]);
    assert!(receiver.start_rekey().is_err());
    assert_eq!(receiver.state(), ChatState::Closed);
}

#[test]
fn runtime_rekey_trigger_counts_real_prepared_outbound_messages() {
    let mut sender = chat(&[7; 256]);
    let noop = json!({"id":0xa82fdd63_u32,"fields":{}});
    for _ in 0..100 {
        sender.prepare_service_action(&noop).unwrap();
    }
    assert!(!sender.rekey_due());
    sender.prepare_service_action(&noop).unwrap();
    assert!(sender.rekey_due());
}

#[test]
fn active_key_cannot_be_replaced_without_the_rekey_lifecycle() {
    let mut receiver = chat(&[7; 256]);
    assert!(receiver.adopt_key(&[8; 256]).is_err());
    assert_eq!(
        receiver
            .receive_encrypted_frame(&frame(&[7; 256], 0, 73))
            .unwrap()
            .text
            .as_deref(),
        Some("0")
    );
}

#[test]
fn oversized_rekey_request_public_value_closes_the_chat() {
    let (_, mut bob) = rekey_pair();
    let request =
        json!({"id":0xf3c9611b_u32,"fields":{"exchange_id":1,"g_a":{"hex":"01".repeat(257)}}});
    assert!(bob.handle_rekey_action(&request).is_err());
    assert_eq!(bob.state(), ChatState::Closed);
}

fn decode_action(key: &[u8; 256], wire: &[u8], x: i64) -> serde_json::Value {
    let body = decrypt_frame(key, wire, x).unwrap();
    schema_secret_tl::decode_object(hex::encode(body), String::new(), 0).unwrap()["object"]["fields"]["message"]["fields"]["action"].clone()
}

#[test]
fn oversized_rekey_accept_public_value_aborts_the_exchange_without_changing_key() {
    let (mut alice, _) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = decode_action(&[7; 256], &request.frame, 0);
    let accept = json!({"id":0x6fe1735b_u32,"fields":{
        "exchange_id":action["fields"]["exchange_id"],"g_b":{"hex":"01".repeat(257)},"key_fingerprint":1
    }});
    let aborts = alice.handle_rekey_action(&accept).unwrap();
    assert_eq!(aborts.len(), 1);
    assert_eq!(
        decode_action(&[7; 256], &aborts[0].frame, 0)["id"],
        0xdd05ec6b_u32
    );
    assert_eq!(alice.state(), ChatState::Ready);
    assert!(alice.start_rekey().unwrap().is_some());
}

#[test]
fn sequence_record_parser_refuses_trailing_uninterpreted_bytes() {
    let mut record = kiaro_secret_chat::ported::sequence::pack_frame(
        json!({"in_seq_no":0,"out_seq_no":1,"layer":73}),
    )
    .unwrap();
    record["body"] = json!(format!("{}00000000", record["body"].as_str().unwrap()));
    let state = json!({"id":7,"is_outbound":false,"in_seq_no":0,"out_seq_no":0,
        "peer_in_seq_no":0,"layer":46,"wrapper_layer":46,"gap_requested":false});
    assert!(kiaro_secret_chat::ported::sequence::preflight(state, record, vec![]).is_err());
}

#[test]
fn tl_extended_length_cannot_encode_a_short_string() {
    assert!(
        schema_secret_tl::decode_object("7a761eb7fe01000061000000".into(), String::new(), 0)
            .is_err()
    );
}

#[test]
fn runtime_rekey_equal_collision_clears_the_outstanding_exchange() {
    let (mut alice, _) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = decode_action(&[7; 256], &request.frame, 0);
    assert!(alice.handle_rekey_action(&action).unwrap().is_empty());
    assert!(alice.start_rekey().unwrap().is_some());
}

#[test]
fn runtime_rekey_larger_signed_exchange_wins_without_overwriting_an_accepted_exchange() {
    let (mut alice, _) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let mut action = decode_action(&[7; 256], &request.frame, 0);
    let mine = action["fields"]["exchange_id"].as_i64().unwrap();
    if let Some(smaller) = mine.checked_sub(1) {
        action["fields"]["exchange_id"] = json!(smaller);
        assert!(alice.handle_rekey_action(&action).unwrap().is_empty());
        assert!(alice.start_rekey().unwrap().is_none());
    }
    if let Some(larger) = mine.checked_add(1) {
        action["fields"]["exchange_id"] = json!(larger);
        let reply = alice.handle_rekey_action(&action).unwrap().remove(0);
        assert_eq!(
            decode_action(&[7; 256], &reply.frame, 0)["fields"]["exchange_id"],
            larger
        );
        action["fields"]["exchange_id"] = json!(i64::MAX);
        assert!(alice.handle_rekey_action(&action).unwrap().is_empty());
    }
}

#[test]
fn runtime_rekey_ignores_unrelated_abort_but_clears_the_matching_exchange() {
    let (mut alice, _) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = decode_action(&[7; 256], &request.frame, 0);
    let mine = action["fields"]["exchange_id"].as_i64().unwrap();
    alice
        .handle_rekey_action(&json!({"id":0xdd05ec6b_u32,"fields":{"exchange_id":mine ^ 1}}))
        .unwrap();
    assert!(alice.start_rekey().unwrap().is_none());
    alice
        .handle_rekey_action(&json!({"id":0xdd05ec6b_u32,"fields":{"exchange_id":mine}}))
        .unwrap();
    assert!(alice.start_rekey().unwrap().is_some());
}

#[test]
fn pending_key_fingerprint_alone_cannot_authorize_a_switch() {
    let (mut alice, mut bob) = rekey_pair();
    let request = alice.start_rekey().unwrap().unwrap();
    let action = receive_action(&mut bob, &request.frame);
    let accept = bob.handle_rekey_action(&action).unwrap().remove(0);
    let action = receive_action(&mut alice, &accept.frame);
    let _commit = alice.handle_rekey_action(&action).unwrap().remove(0);
    let next = alice
        .prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))
        .unwrap();
    let original_lifecycle = bob.key_lifecycle();
    let mut tampered = next.frame.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(bob.receive_encrypted_frames(&tampered).is_err());
    assert_eq!(bob.key_lifecycle(), original_lifecycle);
    assert!(!bob.take_pending_noop());
    assert!(!bob.gap_requested());
}

#[test]
fn service_sequence_overflow_is_rejected_without_advancing_the_counter() {
    let mut sender = chat(&[7; 256]);
    sender.set_out_seq_no(i64::MAX);
    assert!(
        sender
            .prepare_service_action(&json!({"id":0xa82fdd63_u32,"fields":{}}))
            .is_err()
    );
    assert_eq!(sender.out_seq_no(), i64::MAX);
}

#[test]
fn accepted_message_debug_does_not_disclose_plaintext() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let body = wrap_frame(&json!({"message":"private-message-do-not-log"}), 73, 0, 1).unwrap();
    let message = receiver
        .receive_encrypted_frame(&encrypt_frame(&key, &body, 0).unwrap())
        .unwrap();
    assert!(!format!("{message:?}").contains("private-message-do-not-log"));
}

#[test]
fn ingress_budget_rejects_oversized_frames_without_advancing_the_chat() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let body = wrap_frame(&json!({"message":"a".repeat(1024 * 1024)}), 73, 0, 1).unwrap();
    let oversized = encrypt_frame(&key, &body, 0).unwrap();
    assert!(receiver.receive_encrypted_frames(&oversized).is_err());
    assert_eq!(receiver.in_seq_no(), 0);
    assert_eq!(receiver.state(), ChatState::Ready);
    assert_eq!(
        receiver
            .receive_encrypted_frames(&frame(&key, 0, 73))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn ingress_budget_closes_a_chat_when_authenticated_gap_retention_exceeds_16_mib() {
    let key = [7; 256];
    let mut receiver = chat(&key);
    let message = "a".repeat(1024 * 1024 - 4096);
    for sequence in 1..=17 {
        let body = wrap_frame(
            &json!({"random_id":sequence,"message":message}),
            73,
            0,
            sequence * 2 + 1,
        )
        .unwrap();
        let packet = encrypt_frame(&key, &body, 0).unwrap();
        assert!(packet.len() < 1024 * 1024);
        let result = receiver.receive_encrypted_frames(&packet);
        if sequence < 17 {
            assert!(result.unwrap().is_empty());
        } else {
            assert!(result.is_err());
        }
    }
    assert_eq!(receiver.state(), ChatState::Closed);
}

#[test]
fn dh_zero_secret_cannot_produce_a_publicly_known_shared_key() {
    let mut peer = [0u8; 256];
    peer[7] = 1;
    assert!(
        kiaro_secret_chat::protocol::dh_shared_key(&peer, &[0; 256], &dh_prime(), Some(7)).is_err()
    );
    assert!(kiaro_secret_chat::protocol::dh_public_value(&[2], &[0; 256], &dh_prime()).is_err());
}

#[test]
fn oversized_outbound_service_is_rejected_before_reserving_a_sequence_number() {
    let mut sender = chat(&[7; 256]);
    let action = json!({"id":0x0c4f40be_u32,"fields":{"random_ids":vec![1i64; 140_000]}});
    assert!(sender.prepare_service_action(&action).is_err());
    assert_eq!(sender.out_seq_no(), 0);
}

#[test]
fn fingerprint_verification_refuses_noncanonical_key_lengths() {
    use sha1::{Digest, Sha1};
    let fingerprint = i64::from_le_bytes(Sha1::digest([])[12..20].try_into().unwrap());
    assert!(kiaro_secret_chat::protocol::verify_dh_fingerprint(7, &[], fingerprint).is_err());
}

#[test]
fn wrapper_random_bytes_must_meet_the_documented_15_byte_minimum() {
    let key = [7; 256];
    for length in [0, 14, 15] {
        let mut receiver = chat(&key);
        let encoded = schema_secret_tl::encode_object(json!({"id":0x1be31789_u32,"fields":{
            "random_bytes":{"hex":"01".repeat(length)},"layer":73,"in_seq_no":0,"out_seq_no":1,
            "message":{"id":0x73164160_u32,"fields":{"random_id":1,"action":{"id":0xa82fdd63_u32,"fields":{}}}}
        }})).unwrap();
        let packet = encrypt_frame(&key, &hex::decode(encoded).unwrap(), 0).unwrap();
        let result = receiver.receive_encrypted_frames(&packet);
        if length < 15 {
            assert!(result.is_err(), "wrapper nonce length {length}");
            assert_eq!(receiver.in_seq_no(), 0);
        } else {
            assert_eq!(result.unwrap().len(), 1);
        }
        assert_eq!(receiver.state(), ChatState::Ready);
    }
}
