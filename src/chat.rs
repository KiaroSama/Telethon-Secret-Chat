//! Persisted conversation state; public snapshots omit shared keys and payloads.
use crate::{
    crypto::{Direction, SharedKey},
    dh::SecretExponent,
    error::{Error, Result},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatState {
    Requested,
    Waiting,
    Ready,
    Rekeying,
    Suspended,
    Closed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DhRecord {
    pub g: u32,
    pub prime: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum Exchange {
    Requested { id: i64, secret: SecretExponent },
    Accepted { id: i64, key: SharedKey },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outgoing {
    pub seq_no: i32,
    pub random_id: i64,
    pub frame: Vec<u8>,
    pub service: bool,
    pub file: Option<Vec<u8>>,
    pub pending: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub body: Vec<u8>,
    pub file: Option<Vec<u8>>,
    pub date: i32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chat {
    pub id: i32,
    pub access_hash: i64,
    pub peer_user_id: i64,
    pub is_outbound: bool,
    pub state: ChatState,
    pub key: Option<SharedKey>,
    pub previous_key: Option<SharedKey>,
    pub dh: Option<DhRecord>,
    pub handshake_secret: Option<SecretExponent>,
    pub peer_public: Option<Vec<u8>>,
    pub exchange: Option<Exchange>,
    pub initial_key_hash: Option<Vec<u8>>,
    pub in_seq_no: i32,
    pub out_seq_no: i32,
    pub peer_in_seq_no: i32,
    pub capability_layer: i32,
    pub wrapper_layer: i32,
    pub ttl: i32,
    pub gap_end: Option<i32>,
    pub resend_due: Option<(i32, i32)>,
    pub new_key_confirmed: bool,
    pub rekeyed_at: u64,
    pub messages_since_rekey: u64,
    pub gaps: BTreeMap<i32, Envelope>,
    pub outbox: BTreeMap<i32, Outgoing>,
    pub deliveries: BTreeMap<i32, Envelope>,
    pub paused: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSnapshot {
    pub id: i32,
    pub peer_user_id: i64,
    pub is_outbound: bool,
    pub state: ChatState,
    pub ttl: i32,
    pub capability_layer: i32,
    pub key_fingerprint: Option<i64>,
    pub key_hash: Option<Vec<u8>>,
    pub has_previous_key: bool,
    pub exchange_in_progress: bool,
    pub paused: bool,
}
impl Chat {
    pub fn new(
        id: i32,
        access_hash: i64,
        peer_user_id: i64,
        is_outbound: bool,
        state: ChatState,
    ) -> Self {
        Self {
            id,
            access_hash,
            peer_user_id,
            is_outbound,
            state,
            key: None,
            previous_key: None,
            dh: None,
            handshake_secret: None,
            peer_public: None,
            exchange: None,
            initial_key_hash: None,
            in_seq_no: 0,
            out_seq_no: 0,
            peer_in_seq_no: 0,
            capability_layer: 46,
            wrapper_layer: 0,
            ttl: 0,
            gap_end: None,
            resend_due: None,
            new_key_confirmed: false,
            rekeyed_at: 0,
            messages_since_rekey: 0,
            gaps: BTreeMap::new(),
            outbox: BTreeMap::new(),
            deliveries: BTreeMap::new(),
            paused: false,
        }
    }
    pub fn snapshot(&self) -> ChatSnapshot {
        ChatSnapshot {
            id: self.id,
            peer_user_id: self.peer_user_id,
            is_outbound: self.is_outbound,
            state: self.state,
            ttl: self.ttl,
            capability_layer: self.capability_layer,
            key_fingerprint: self.key.as_ref().map(SharedKey::fingerprint),
            key_hash: self.initial_key_hash.clone(),
            has_previous_key: self.previous_key.is_some(),
            exchange_in_progress: self.exchange.is_some(),
            paused: self.paused,
        }
    }
    pub fn out_direction(&self) -> Direction {
        if self.is_outbound {
            Direction::Initiator
        } else {
            Direction::Responder
        }
    }
    pub fn in_direction(&self) -> Direction {
        if self.is_outbound {
            Direction::Responder
        } else {
            Direction::Initiator
        }
    }
    pub fn require_ready(&self) -> Result<()> {
        if self.paused {
            return Err(Error::Paused);
        }
        if !matches!(self.state, ChatState::Ready | ChatState::Rekeying) || self.key.is_none() {
            return Err(Error::ChatState);
        }
        Ok(())
    }
    pub fn adopt_key(&mut self, key: SharedKey) {
        if self.initial_key_hash.is_none() {
            self.initial_key_hash = Some(crate::visualization::key_hash(&key).to_vec());
        }
        self.key = Some(key);
    }
    pub fn close(&mut self) {
        self.state = ChatState::Closed;
        self.key = None;
        self.previous_key = None;
        self.exchange = None;
        self.handshake_secret = None;
        self.peer_public = None;
        self.dh = None;
        self.gaps.clear();
        self.outbox.clear();
        self.deliveries.clear();
        self.gap_end = None;
        self.resend_due = None;
    }
    pub fn validate(&self) -> Result<()> {
        if self.id <= 0
            || self.peer_user_id <= 0
            || self.in_seq_no < 0
            || self.out_seq_no < 0
            || self.peer_in_seq_no < 0
            || self.peer_in_seq_no > self.out_seq_no
            || self.out_seq_no > i32::MAX / 2
            || self.in_seq_no > i32::MAX / 2
            || self.ttl < 0
            || self.capability_layer < 46
            || (self.wrapper_layer != 0 && self.wrapper_layer < 46)
            || self.wrapper_layer > self.capability_layer
            || self.gaps.len() > 1000
            || self
                .initial_key_hash
                .as_ref()
                .is_some_and(|h| h.len() != 36)
        {
            return Err(Error::InvalidState);
        }
        if matches!(self.state, ChatState::Ready | ChatState::Rekeying)
            && (self.key.is_none() || self.dh.is_none())
        {
            return Err(Error::InvalidState);
        }
        if self.state == ChatState::Closed
            && (self.key.is_some()
                || self.previous_key.is_some()
                || self.exchange.is_some()
                || self.handshake_secret.is_some()
                || !self.outbox.is_empty()
                || !self.gaps.is_empty()
                || !self.deliveries.is_empty())
        {
            return Err(Error::InvalidState);
        }
        for (seq, item) in &self.outbox {
            if *seq != item.seq_no
                || *seq < 0
                || *seq / 2 >= self.out_seq_no
                || *seq % 2 != i32::from(self.is_outbound)
                || item.frame.len() < 40
                || item.frame.len() > crate::crypto::MAX_FRAME_LENGTH
                || (item.frame.len() - 24) % 16 != 0
            {
                return Err(Error::InvalidState);
            }
        }
        for (seq, item) in self.gaps.iter().chain(&self.deliveries) {
            let wrapper = crate::framing::unwrap(&item.body).map_err(|_| Error::InvalidState)?;
            if *seq < 0 || wrapper.out_seq_no / 2 != *seq {
                return Err(Error::InvalidState);
            }
        }
        Ok(())
    }
}
