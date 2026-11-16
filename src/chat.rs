//! Persisted conversation state; public snapshots omit shared keys and payloads.
mod validation;
use crate::{
    crypto::{Direction, SharedKey},
    dh::SecretExponent,
    error::{CloseReason, Error, Result},
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
    #[serde(default)]
    pub body: Vec<u8>,
    #[serde(default)]
    pub failures: u8,
    pub service: bool,
    pub file: Option<Vec<u8>>,
    pub pending: bool,
    /// Distinguishes a new retransmission obligation from an older in-flight
    /// send of the exact same ciphertext. Old native snapshots start at zero.
    #[serde(default)]
    pub obligation_generation: u64,
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
pub struct ServiceDelivery {
    pub envelope: Envelope,
    pub applied: Option<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chat {
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub admin_id: Option<i64>,
    #[serde(default)]
    pub participant_id: Option<i64>,
    #[serde(default)]
    pub closed_reason: Option<CloseReason>,
    pub id: i32,
    pub access_hash: i64,
    pub peer_user_id: i64,
    pub is_outbound: bool,
    pub state: ChatState,
    pub key: Option<SharedKey>,
    pub previous_key: Option<SharedKey>,
    pub dh: Option<DhRecord>,
    #[serde(default)]
    pub handshake_secret: Option<SecretExponent>,
    #[serde(default)]
    pub peer_public: Option<Vec<u8>>,
    pub exchange: Option<Exchange>,
    #[serde(default)]
    pub initial_key_hash: Option<Vec<u8>>,
    pub in_seq_no: i32,
    pub out_seq_no: i32,
    pub peer_in_seq_no: i32,
    pub capability_layer: i32,
    pub wrapper_layer: i32,
    pub ttl: i32,
    #[serde(default)]
    pub gap_end: Option<i32>,
    pub resend_due: Option<(i32, i32)>,
    #[serde(default)]
    pub new_key_confirmed: bool,
    pub rekeyed_at: u64,
    pub messages_since_rekey: u64,
    pub gaps: BTreeMap<i32, Envelope>,
    pub outbox: BTreeMap<i32, Outgoing>,
    pub deliveries: BTreeMap<i32, Envelope>,
    #[serde(default)]
    pub service_deliveries: BTreeMap<i32, ServiceDelivery>,
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
            created_at: 0,
            admin_id: None,
            participant_id: None,
            closed_reason: None,
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
            service_deliveries: BTreeMap::new(),
            paused: false,
        }
    }
    /// Create a chat in the initial state corresponding to its role.
    pub fn for_role(id: i32, access_hash: i64, peer_user_id: i64, is_outbound: bool) -> Self {
        Self::new(
            id,
            access_hash,
            peer_user_id,
            is_outbound,
            if is_outbound {
                ChatState::Requested
            } else {
                ChatState::Waiting
            },
        )
    }
    pub fn transition_to(&mut self, target: ChatState) -> Result<()> {
        if self.state == ChatState::Closed {
            return Err(Error::ChatEnded {
                chat_id: self.id,
                reason: self.closed_reason.unwrap_or(CloseReason::AlreadyClosed),
            });
        }
        let legal = matches!(
            (self.state, target),
            (
                ChatState::Requested | ChatState::Waiting,
                ChatState::Ready | ChatState::Closed
            ) | (ChatState::Ready, ChatState::Rekeying | ChatState::Closed)
                | (ChatState::Rekeying, ChatState::Ready | ChatState::Closed)
        );
        if !legal {
            return Err(Error::ChatNotSendable {
                chat_id: self.id,
                state: self.state,
            });
        }
        if target == ChatState::Closed {
            self.close();
        } else {
            self.state = target;
        }
        Ok(())
    }
    pub fn require_sendable(&self) -> Result<()> {
        if self.state == ChatState::Closed {
            return Err(Error::ChatEnded {
                chat_id: self.id,
                reason: self.closed_reason.unwrap_or(CloseReason::AlreadyClosed),
            });
        }
        if self.paused {
            return Err(Error::Paused);
        }
        if !matches!(self.state, ChatState::Ready | ChatState::Rekeying) || self.key.is_none() {
            return Err(Error::ChatNotSendable {
                chat_id: self.id,
                state: self.state,
            });
        }
        Ok(())
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
    pub fn adopt_key_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.adopt_key(SharedKey::from_slice(bytes)?)
    }
    pub fn adopt_key(&mut self, key: SharedKey) -> Result<()> {
        if self.state == ChatState::Closed {
            return Err(Error::ChatEnded {
                chat_id: self.id,
                reason: self.closed_reason.unwrap_or(CloseReason::AlreadyClosed),
            });
        }
        if self.initial_key_hash.is_none() {
            self.initial_key_hash = Some(crate::visualization::key_hash(&key).to_vec());
        }
        self.key = Some(key);
        if matches!(self.state, ChatState::Requested | ChatState::Waiting) {
            self.state = ChatState::Ready;
        }
        Ok(())
    }
    pub fn close_with_reason(&mut self, reason: CloseReason) {
        if self.closed_reason.is_none() {
            self.closed_reason = Some(reason);
        }
        self.close();
    }
    pub fn close(&mut self) {
        if self.closed_reason.is_none() {
            self.closed_reason = Some(CloseReason::Application);
        }
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
        self.service_deliveries.clear();
        self.gap_end = None;
        self.resend_due = None;
    }
}

impl std::fmt::Display for ChatState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Requested => "requested",
            Self::Waiting => "pending",
            Self::Ready => "ready",
            Self::Rekeying => "rekeying",
            Self::Suspended => "suspended",
            Self::Closed => "closed",
        })
    }
}
impl std::fmt::Debug for Chat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.snapshot().fmt(formatter)
    }
}
