#![forbid(unsafe_code)]

use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord, Exchange},
    crypto::{Direction, SharedKey, encrypt_frame},
    enums, framing,
    protocol::Protocol,
    storage::{MemoryStorage, State, Storage},
    types,
};

const CHAT_ID: i32 = 7;
const AUTHORIZATION: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn key(seed: u8) -> SharedKey {
    let bytes: [u8; 256] =
        std::array::from_fn(|index| (index as u8).wrapping_mul(37).wrapping_add(seed));
    SharedKey::from_slice(&bytes).unwrap()
}

fn receiver() -> Protocol<MemoryStorage> {
    let mut protocol = Protocol::open(MemoryStorage::default(), AUTHORIZATION.into()).unwrap();
    protocol.start().unwrap();
    let mut chat = Chat::for_role(CHAT_ID, 88, 1000, false);
    chat.adopt_key(key(7)).unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: hex::decode(include_str!("../fixtures/dh-prime.hex").trim()).unwrap(),
    });
    protocol.insert_chat(chat).unwrap();
    protocol
}

fn accepted_receiver() -> Protocol<MemoryStorage> {
    let mut protocol = receiver();
    protocol
        .mutate_chat(CHAT_ID, |chat| {
            chat.state = ChatState::Rekeying;
            chat.exchange = Some(Exchange::Accepted {
                id: 99,
                key: key(19),
            });
            Ok(())
        })
        .unwrap();
    protocol
}

fn message(raw: i32) -> enums::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: i64::from(raw) + 100,
        ttl: 0,
        message: format!("synthetic message {raw}"),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}

fn frame(
    raw: i32,
    acknowledgement: i32,
    layer: i32,
    shared_key: &SharedKey,
    message: enums::DecryptedMessage,
) -> Vec<u8> {
    let body = framing::wrap(message, layer, acknowledgement * 2, raw * 2 + 1)
        .unwrap()
        .to_bytes();
    encrypt_frame(shared_key, &body, Direction::Initiator).unwrap()
}

fn text_frame(raw: i32, acknowledgement: i32, layer: i32, shared_key: &SharedKey) -> Vec<u8> {
    frame(raw, acknowledgement, layer, shared_key, message(raw))
}

fn state_marker(protocol: &Protocol<MemoryStorage>) -> serde_json::Value {
    serde_json::to_value(protocol.chat_record(CHAT_ID).unwrap()).unwrap()
}

fn reopen(protocol: &Protocol<MemoryStorage>) -> Protocol<MemoryStorage> {
    let mut state = State::empty(AUTHORIZATION.into()).unwrap();
    let encoded = serde_json::to_vec(&protocol.chat_record(CHAT_ID).unwrap()).unwrap();
    let chat = serde_json::from_slice::<Chat>(&encoded).unwrap();
    state.chats.insert(CHAT_ID, chat);
    let mut storage = MemoryStorage::default();
    storage.commit(&state).unwrap();
    let mut restored = Protocol::open(storage, AUTHORIZATION.into()).unwrap();
    restored.start().unwrap();
    restored
}

#[test]
fn old_key_cannot_authorize_future_sequence_after_new_key_arrival() {
    let mut protocol = accepted_receiver();
    protocol
        .receive(CHAT_ID, &text_frame(1, 0, 144, &key(19)), None, 1)
        .unwrap();
    let before = state_marker(&protocol);
    assert!(
        protocol
            .receive(CHAT_ID, &text_frame(2, 0, 144, &key(7)), None, 1)
            .is_err(),
        "the held previous key must not authorize sequences after the authenticated key switch"
    );
    assert!(
        before == state_marker(&protocol),
        "rejected input changed state"
    );
}

#[test]
fn key_switch_rejects_already_buffered_future_old_key_frame() {
    let mut protocol = accepted_receiver();
    protocol
        .receive(CHAT_ID, &text_frame(2, 0, 144, &key(7)), None, 1)
        .unwrap();
    let before = state_marker(&protocol);
    assert!(
        protocol
            .receive(CHAT_ID, &text_frame(1, 0, 144, &key(19)), None, 1)
            .is_err(),
        "new-key traffic must validate the provenance of future buffered packets"
    );
    assert!(
        before == state_marker(&protocol),
        "rejected switch changed state"
    );
}

#[test]
fn buffered_duplicate_with_conflicting_shared_key_is_rejected() {
    let mut protocol = accepted_receiver();
    protocol
        .receive(CHAT_ID, &text_frame(1, 0, 144, &key(7)), None, 1)
        .unwrap();
    let before = state_marker(&protocol);
    assert!(
        protocol
            .receive(CHAT_ID, &text_frame(1, 0, 144, &key(19)), None, 1)
            .is_err(),
        "different authenticated keys for one pending sequence are not an ordinary replay"
    );
    assert!(
        before == state_marker(&protocol),
        "conflicting replay changed state"
    );
}

#[test]
fn key_switch_boundary_survives_serialized_restart() {
    let mut protocol = accepted_receiver();
    protocol
        .receive(CHAT_ID, &text_frame(1, 0, 144, &key(19)), None, 1)
        .unwrap();
    let mut restored = reopen(&protocol);
    let before = state_marker(&restored);
    assert!(
        restored
            .receive(CHAT_ID, &text_frame(2, 0, 144, &key(7)), None, 1)
            .is_err(),
        "restart must preserve the previous key's incoming sequence boundary"
    );
    assert!(
        before == state_marker(&restored),
        "rejected input changed restored state"
    );
}

#[test]
fn valid_old_key_gap_before_the_switch_drains_after_restart() {
    let mut protocol = accepted_receiver();
    protocol
        .receive(CHAT_ID, &text_frame(2, 0, 144, &key(19)), None, 1)
        .unwrap();
    let mut restored = reopen(&protocol);
    for raw in [0, 1] {
        restored
            .receive(CHAT_ID, &text_frame(raw, 0, 144, &key(7)), None, 1)
            .unwrap();
    }
    assert_eq!(restored.pending_deliveries(CHAT_ID).unwrap(), vec![0, 1, 2]);
    let chat = restored.chat_record(CHAT_ID).unwrap();
    assert_eq!(chat.in_seq_no, 3);
    assert!(chat.gaps.is_empty());
    assert!(chat.previous_key.is_none());
}

#[test]
fn later_new_key_gap_does_not_retain_a_settled_previous_key() {
    let mut protocol = accepted_receiver();
    for raw in [1, 3] {
        protocol
            .receive(CHAT_ID, &text_frame(raw, 0, 144, &key(19)), None, 1)
            .unwrap();
    }
    protocol
        .receive(CHAT_ID, &text_frame(0, 0, 144, &key(7)), None, 1)
        .unwrap();
    let chat = protocol.chat_record(CHAT_ID).unwrap();
    assert_eq!(chat.in_seq_no, 2);
    assert_eq!(chat.gaps.keys().copied().collect::<Vec<_>>(), vec![3]);
    assert!(
        chat.previous_key.is_none(),
        "a later gap containing only new-key traffic must not extend the previous key's lifetime"
    );
}

#[test]
fn successor_acknowledgement_is_checked_before_immediate_resend() {
    let mut protocol = receiver();
    for text in ["first outgoing", "second outgoing"] {
        protocol
            .send_message(CHAT_ID, text.into(), None, None)
            .unwrap();
    }
    protocol
        .receive(CHAT_ID, &text_frame(2, 1, 144, &key(7)), None, 1)
        .unwrap();
    let request = types::DecryptedMessageService {
        random_id: 301,
        action: types::DecryptedMessageActionResend {
            start_seq_no: 0,
            end_seq_no: 0,
        }
        .into(),
    };
    let before = state_marker(&protocol);
    assert!(
        protocol
            .receive(CHAT_ID, &frame(1, 2, 144, &key(7), request.into()), None, 1)
            .is_err(),
        "a wrapper contradicting its buffered successor must not schedule retransmission"
    );
    assert!(
        before == state_marker(&protocol),
        "invalid Resend changed obligations"
    );
}

#[test]
fn buffered_neighbor_layer_regression_is_rejected_on_arrival() {
    let mut protocol = receiver();
    protocol
        .receive(CHAT_ID, &text_frame(2, 0, 73, &key(7)), None, 1)
        .unwrap();
    let before = state_marker(&protocol);
    assert!(
        protocol
            .receive(CHAT_ID, &text_frame(1, 0, 144, &key(7)), None, 1)
            .is_err(),
        "a predecessor's layer cannot exceed its already buffered successor's layer"
    );
    assert!(
        before == state_marker(&protocol),
        "invalid buffered ordering changed state"
    );
}

#[test]
fn final_representable_outgoing_sequence_can_be_prepared_once() {
    let mut protocol = receiver();
    protocol
        .mutate_chat(CHAT_ID, |chat| {
            chat.out_seq_no = i32::MAX / 2;
            Ok(())
        })
        .unwrap();
    let last = protocol.send_message(CHAT_ID, "final frame".into(), None, None);
    assert!(
        last.is_ok(),
        "the final nonnegative i32 wire index is still representable"
    );
    assert_eq!(last.unwrap().outgoing.seq_no, i32::MAX - 1);
    assert_eq!(
        protocol.chat_record(CHAT_ID).unwrap().out_seq_no,
        i32::MAX / 2 + 1
    );
    assert!(
        protocol
            .send_message(CHAT_ID, "overflow".into(), None, None)
            .is_err()
    );
}

#[test]
fn final_representable_incoming_sequence_can_be_accepted_once() {
    let mut protocol = receiver();
    protocol
        .mutate_chat(CHAT_ID, |chat| {
            chat.in_seq_no = i32::MAX / 2;
            Ok(())
        })
        .unwrap();
    let last = text_frame(i32::MAX / 2, 0, 144, &key(7));
    assert!(
        protocol.receive(CHAT_ID, &last, None, 1).is_ok(),
        "the final incoming wire index must advance the local count once"
    );
    assert_eq!(
        protocol.chat_record(CHAT_ID).unwrap().in_seq_no,
        i32::MAX / 2 + 1
    );
    assert_eq!(
        protocol.pending_deliveries(CHAT_ID).unwrap(),
        vec![i32::MAX / 2]
    );
}
