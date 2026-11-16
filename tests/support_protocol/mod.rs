#![allow(dead_code, clippy::unwrap_used)]
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord, Envelope},
    crypto::{Direction, SharedKey},
    protocol::Protocol,
    storage::{MemoryStorage, State, Storage},
    types,
};
pub fn key() -> SharedKey {
    SharedKey::from_slice(&[7; 256]).unwrap()
}
pub fn chat(outbound: bool, sent: i32) -> Chat {
    let mut chat = Chat::for_role(7, 8, 9, outbound);
    chat.dh = Some(DhRecord {
        g: 2,
        prime: hex::decode(include_str!("../fixtures/dh-prime.hex").trim()).unwrap(),
    });
    chat.adopt_key(key()).unwrap();
    chat.out_seq_no = sent;
    chat
}
pub fn message(raw: i32, text: &str) -> kiaro_secret_chat::enums::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: i64::from(raw),
        ttl: 0,
        message: text.into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}
pub fn wrap(in_seq: i32, out_seq: i32, layer: i32) -> types::DecryptedMessageLayer {
    types::DecryptedMessageLayer {
        random_bytes: vec![0; 31],
        layer,
        in_seq_no: in_seq,
        out_seq_no: out_seq,
        message: message(out_seq / 2, "public fixture"),
    }
}
pub fn envelope(raw: i32, echo: i32, layer: i32, outbound: bool) -> Envelope {
    Envelope {
        body: kiaro_secret_chat::enums::DecryptedMessageLayer::from(wrap(
            2 * echo + i32::from(outbound),
            2 * raw + i32::from(!outbound),
            layer,
        ))
        .to_bytes(),
        file: None,
        date: 1,
    }
}
pub fn from_wrapper(wrapper: types::DecryptedMessageLayer) -> Envelope {
    Envelope {
        body: kiaro_secret_chat::enums::DecryptedMessageLayer::from(wrapper).to_bytes(),
        file: None,
        date: 1,
    }
}
pub fn protocol() -> Protocol<MemoryStorage> {
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    let mut store = MemoryStorage::default();
    store.commit(&state).unwrap();
    let mut p = Protocol::open(store, "11".repeat(32)).unwrap();
    p.start().unwrap();
    p
}
pub fn frame(body: &[u8]) -> Vec<u8> {
    kiaro_secret_chat::crypto::encrypt_frame(&key(), body, Direction::Responder).unwrap()
}
pub fn ids(result: kiaro_secret_chat::sequence::Accepted) -> Vec<i64> {
    result
        .ready
        .iter()
        .map(|(_, row)| {
            kiaro_secret_chat::framing::unwrap(&row.body)
                .unwrap()
                .message
                .random_id()
        })
        .collect()
}
pub fn assert_ready(chat: &Chat) {
    assert_eq!(chat.state, ChatState::Ready);
}
