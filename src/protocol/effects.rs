use crate::{chat::Outgoing, enums};
/// A network effect carries protected bytes and intentionally has no Debug.
#[derive(Clone)]
pub struct SendEffect {
    pub chat_id: i32,
    pub access_hash: i64,
    pub outgoing: Outgoing,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    MessageAvailable {
        chat_id: i32,
        sequence: i32,
        random_id: i64,
    },
    MessageAcknowledged {
        chat_id: i32,
        random_ids: Vec<i64>,
    },
    LayerChanged {
        chat_id: i32,
        layer: i32,
    },
}
#[derive(Clone)]
pub struct ServiceEvent {
    pub chat_id: i32,
    pub seq_no: i32,
    pub action: enums::DecryptedMessageAction,
    pub applied: bool,
}
#[derive(Default)]
pub struct Effects {
    pub sends: Vec<SendEffect>,
    pub events: Vec<Event>,
}
