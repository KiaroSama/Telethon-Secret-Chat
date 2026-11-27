mod helpers;
mod receive_guard;
mod snapshot;

pub use helpers::*;
pub(crate) use snapshot::PersistedProtocolState;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::errors::SecretChatError;

const INITIAL_REMOTE_LAYER: i64 = 46;
const SHARED_KEY_LEN: usize = 256;
const MIN_PADDING_LEN: usize = 12;
const WRAPPER_CONSTRUCTOR_ID: u32 = 0x1be3_1789;
const DECRYPTED_MESSAGE_73_ID: u32 = 0x91cc_4674;

/// Local resource ceiling for one complete encrypted frame, including its header.
pub const MAX_ENCRYPTED_FRAME_LEN: usize = 1024 * 1024;
/// Local ceiling for the serialized plaintext bodies held behind sequence gaps.
pub const MAX_PENDING_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    Initiator,
    Acceptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatState {
    Requested,
    Offered,
    Ready,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum KeyLifecycleState {
    Unset,
    Active { fingerprint: i64 },
    Rekeying { fingerprint: i64, exchange_id: i64 },
    Zeroized,
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(crate) struct KeySlot(pub(crate) [u8; SHARED_KEY_LEN]);

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedServiceMessage {
    pub random_id: i64,
    /// Raw counter reserved by this preparation; the frame contains its wire form.
    pub out_seq_no: i64,
    pub frame: Vec<u8>,
}

impl std::fmt::Debug for PreparedServiceMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedServiceMessage")
            .field("out_seq_no", &self.out_seq_no)
            .field("frame_length", &self.frame.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcceptedMessage {
    pub layer: i64,
    pub in_seq_no: i64,
    pub out_seq_no: i64,
    pub message_type: String,
    pub text: Option<String>,
    pub raw_inner_bytes: Vec<u8>,
    pub random_id: i64,
    pub action: Option<Value>,
}

impl std::fmt::Debug for AcceptedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptedMessage")
            .field("layer", &self.layer)
            .field("in_seq_no", &self.in_seq_no)
            .field("out_seq_no", &self.out_seq_no)
            .field("message_type", &self.message_type)
            .field("text_length", &self.text.as_ref().map(String::len))
            .field("raw_inner_length", &self.raw_inner_bytes.len())
            .field("has_action", &self.action.is_some())
            .finish()
    }
}

/// Core MTProto 2.0 secret-chat state aligned with `specs/019-rust-anything-full-rewrite/data-model.md`.
/// Clones support atomic manager transactions; every cloned key slot zeroizes on drop.
#[derive(Clone)]
pub struct SecretChat {
    chat_id: i64,
    access_hash: i64,
    peer_user_id: i64,
    caller_auth_id: String,
    role: ChatRole,
    state: ChatState,
    negotiated_layer: i64,
    wrapper_layer: i64,
    in_seq_no: i64,
    out_seq_no: i64,
    peer_in_seq_no: i64,
    ttl: i32,
    pub(crate) key_lifecycle: KeyLifecycleState,
    pub(crate) active_key: Option<KeySlot>,
    pub(crate) rekey: crate::ported::rekey::RuntimeRekey,
    pending_frames: Vec<Value>,
    pending_frame_keys: BTreeMap<i64, i64>,
    ready_messages: Vec<AcceptedMessage>,
    gap_requested: bool,
    gap_end: Option<i64>,
    resend_due: Option<(i64, i64)>,
    resend_request: Option<(i64, i64)>,
    immediate_actions: Vec<Value>,
}

impl SecretChat {
    pub fn new(
        chat_id: i64,
        access_hash: i64,
        peer_user_id: i64,
        caller_auth_id: impl Into<String>,
        role: ChatRole,
    ) -> Self {
        let initial_state = match role {
            ChatRole::Initiator => ChatState::Requested,
            ChatRole::Acceptor => ChatState::Offered,
        };
        Self {
            chat_id,
            access_hash,
            peer_user_id,
            caller_auth_id: caller_auth_id.into(),
            role,
            state: initial_state,
            negotiated_layer: INITIAL_REMOTE_LAYER,
            wrapper_layer: INITIAL_REMOTE_LAYER,
            in_seq_no: 0,
            out_seq_no: 0,
            peer_in_seq_no: 0,
            ttl: 0,
            key_lifecycle: KeyLifecycleState::Unset,
            active_key: None,
            rekey: crate::ported::rekey::RuntimeRekey::default(),
            pending_frames: Vec::new(),
            pending_frame_keys: BTreeMap::new(),
            ready_messages: Vec::new(),
            gap_requested: false,
            gap_end: None,
            resend_due: None,
            resend_request: None,
            immediate_actions: Vec::new(),
        }
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    pub fn access_hash(&self) -> i64 {
        self.access_hash
    }

    pub fn ttl(&self) -> i32 {
        self.ttl
    }

    pub(crate) fn set_persisted_ttl(&mut self, ttl: i32) {
        self.ttl = ttl;
    }

    pub fn peer_user_id(&self) -> i64 {
        self.peer_user_id
    }

    pub fn caller_auth_id(&self) -> &str {
        &self.caller_auth_id
    }

    pub fn role(&self) -> ChatRole {
        self.role
    }

    pub fn is_outbound(&self) -> bool {
        self.role == ChatRole::Initiator
    }

    pub fn state(&self) -> ChatState {
        self.state
    }

    pub fn negotiated_layer(&self) -> i64 {
        self.negotiated_layer
    }

    pub fn wrapper_layer(&self) -> i64 {
        self.wrapper_layer
    }

    pub fn gap_requested(&self) -> bool {
        self.gap_requested
    }

    pub fn resend_due(&self) -> Option<(i64, i64)> {
        self.resend_due
    }

    /// Consume a newly discovered gap request after retaining its outgoing frame.
    pub fn take_resend_request(&mut self) -> Option<(i64, i64)> {
        self.resend_request.take()
    }

    /// Drain messages recovered by the compatibility single-message receive API.
    pub fn take_ready_messages(&mut self) -> Vec<AcceptedMessage> {
        std::mem::take(&mut self.ready_messages)
    }

    /// Resend actions bypass gaps, and must be handled before pruning the outbox.
    pub fn take_immediate_actions(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.immediate_actions)
    }

    pub fn in_seq_no(&self) -> i64 {
        self.in_seq_no
    }

    pub fn out_seq_no(&self) -> i64 {
        self.out_seq_no
    }

    pub fn set_out_seq_no(&mut self, value: i64) {
        if value > self.out_seq_no {
            let count = value.abs_diff(self.out_seq_no);
            self.rekey.messages = self.rekey.messages.saturating_add(count);
            self.rekey.outgoing = self.rekey.outgoing.saturating_add(count);
        }
        self.out_seq_no = value;
    }

    pub fn peer_in_seq_no(&self) -> i64 {
        self.peer_in_seq_no
    }

    pub fn key_lifecycle(&self) -> KeyLifecycleState {
        self.key_lifecycle
    }

    pub fn in_x(&self) -> i64 {
        if self.is_outbound() { 8 } else { 0 }
    }

    pub fn out_x(&self) -> i64 {
        if self.is_outbound() { 0 } else { 8 }
    }

    pub fn adopt_key(&mut self, key: &[u8]) -> Result<i64, SecretChatError> {
        if self.state == ChatState::Closed {
            return Err(SecretChatError::ChatClosed {
                chat_id: self.chat_id,
                reason: "chat is closed".into(),
            });
        }
        if self.active_key.is_some() {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id),
                reason: "an active key must be replaced through the rekey lifecycle".into(),
            });
        }
        if key.len() != SHARED_KEY_LEN {
            return Err(SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id),
                reason: "shared key must be 256 bytes".into(),
            });
        }
        let fingerprint = key_fingerprint(key)?;
        let mut buf = [0u8; SHARED_KEY_LEN];
        buf.copy_from_slice(key);
        self.active_key = Some(KeySlot(buf));
        self.key_lifecycle = KeyLifecycleState::Active { fingerprint };
        self.state = ChatState::Ready;
        self.rekey.started = std::time::Instant::now();
        Ok(fingerprint)
    }

    pub fn close(&mut self) {
        self.active_key = None;
        self.rekey = crate::ported::rekey::RuntimeRekey::default();
        self.key_lifecycle = KeyLifecycleState::Zeroized;
        self.state = ChatState::Closed;
        self.pending_frames.clear();
        self.pending_frame_keys.clear();
        self.ready_messages.clear();
        self.immediate_actions.clear();
        self.gap_requested = false;
        self.gap_end = None;
        self.resend_due = None;
        self.resend_request = None;
    }

    pub fn receive_encrypted_frame(
        &mut self,
        frame: &[u8],
    ) -> Result<AcceptedMessage, SecretChatError> {
        let mut accepted = self.receive_encrypted_frames(frame)?;
        if accepted.is_empty() {
            return Err(SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id),
                reason: "duplicate or out-of-order frame was not accepted".into(),
            });
        }
        let first = accepted.remove(0);
        self.ready_messages.extend(accepted);
        Ok(first)
    }

    /// Accept a frame and return every message whose preceding gap has closed.
    /// Authenticated queued messages and replays return an empty batch.
    pub fn receive_encrypted_frames(
        &mut self,
        frame: &[u8],
    ) -> Result<Vec<AcceptedMessage>, SecretChatError> {
        if self.state == ChatState::Closed {
            return Err(SecretChatError::ChatClosed {
                chat_id: self.chat_id,
                reason: "chat is closed".into(),
            });
        }
        if frame.len() > MAX_ENCRYPTED_FRAME_LEN {
            return Err(SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id),
                reason: "the encrypted frame exceeds the local size limit".into(),
            });
        }
        if self.active_key.is_none() {
            return Err(SecretChatError::ChatNotReady {
                chat_id: self.chat_id,
                state: format!("{:?}", self.state).to_lowercase(),
            });
        }
        let (key_slot, key_bytes) = self.select_receiving_key(frame)?;

        let chat_id = self.chat_id;
        let rejected = |reason| SecretChatError::MessageRejected {
            chat_id: Some(chat_id),
            reason,
        };
        let decrypted_body = Zeroizing::new(decrypt_frame(&*key_bytes, frame, self.in_x())?);
        let fingerprint = key_fingerprint(&*key_bytes)?;
        if self.authenticated_replay(&decrypted_body, fingerprint)? {
            return Ok(Vec::new());
        }
        let unwrapped = crate::ported::framing::unwrap(decrypted_body.to_vec(), Some(chat_id))
            .map_err(|err| rejected(sanitize_error_reason(&err)))?;
        if unwrapped.get("error_type").is_some() {
            if unwrapped["fatal"] == true {
                self.close();
            }
            return Err(rejected(
                unwrapped["reason"]
                    .as_str()
                    .unwrap_or("invalid message wrapper")
                    .to_owned(),
            ));
        }

        let state_json = json!({
            "id": self.chat_id,
            "is_outbound": self.is_outbound(),
            "in_seq_no": self.in_seq_no,
            "out_seq_no": self.out_seq_no,
            "peer_in_seq_no": self.peer_in_seq_no,
            "layer": self.negotiated_layer,
            "wrapper_layer": self.wrapper_layer,
            "gap_requested": self.gap_requested,
            "gap_end": self.gap_end,
            "resend_due": self.resend_due.map(|span| [span.0, span.1]),
        });
        let mut frame_json = json!({
            "body": hex::encode(&*decrypted_body),
            "file": Value::Null,
        });
        match crate::ported::sequence::preflight(
            state_json.clone(),
            frame_json.clone(),
            self.pending_frames.clone(),
        ) {
            Ok(false) => return Ok(Vec::new()),
            Ok(true) => (),
            Err(error) => {
                self.close();
                return Err(rejected(sanitize_error_reason(&error)));
            }
        }
        if unwrapped["out_seq_no"]
            .as_i64()
            .is_some_and(|wire| wire / 2 > self.in_seq_no)
        {
            let pending_bytes = self.pending_frames.iter().fold(0usize, |total, record| {
                total.saturating_add(record["body"].as_str().map_or(0, |body| body.len() / 2))
            });
            if pending_bytes.saturating_add(decrypted_body.len()) > MAX_PENDING_FRAME_BYTES {
                self.close();
                return Err(rejected(
                    "the incoming gap exceeds the local byte limit".into(),
                ));
            }
        }
        let mut decoded = crate::ported::schema_secret_tl::decode_object(
            hex::encode(&*decrypted_body),
            "DecryptedMessageLayer".into(),
            0,
        )
        .map_err(|err| rejected(sanitize_error_reason(&err)))?;
        let action = decoded["object"]["fields"]["message"]["fields"]
            .get("action")
            .cloned();
        let sequence = unwrapped["out_seq_no"]
            .as_i64()
            .ok_or_else(|| rejected("invalid authenticated outgoing counter".into()))?
            / 2;
        let key_cutover =
            match self.received_key_cutover(sequence, fingerprint, key_slot, action.as_ref()) {
                Ok(cutover) => cutover,
                Err(error) => {
                    self.close();
                    return Err(error);
                }
            };
        let announced = action
            .as_ref()
            .filter(|action| action["id"] == 0xf3048883_u32)
            .and_then(|action| action["fields"]["layer"].as_i64())
            .unwrap_or(self.negotiated_layer);
        let immediate = action.filter(|action| action["id"] == 0x511110b0_u32);
        if immediate.is_some() {
            decoded["object"]["fields"]["message"]["fields"]["action"] =
                json!({"id":0xa82fdd63_u32,"fields":{}});
            frame_json["body"] = json!(
                crate::ported::schema_secret_tl::encode_object(decoded["object"].clone())
                    .map_err(|err| rejected(sanitize_error_reason(&err)))?
            );
        }
        let seq_result = match crate::ported::sequence::accept(
            state_json,
            frame_json,
            self.pending_frames.clone(),
        ) {
            Ok(result) => result,
            Err(error) => {
                self.close();
                return Err(rejected(sanitize_error_reason(&error)));
            }
        };
        let ready = seq_result["ready"].as_array().cloned().unwrap_or_default();
        let new_state = &seq_result["state"];
        let accepted = ready
            .iter()
            .map(|record| accepted_record(record, chat_id))
            .collect::<Result<Vec<_>, _>>()?;

        // Commit state updates only after every cryptographic, schema, and sequence check succeeded.
        self.in_seq_no = new_state["in_seq_no"].as_i64().unwrap_or(self.in_seq_no);
        self.peer_in_seq_no = new_state["peer_in_seq_no"]
            .as_i64()
            .unwrap_or(self.peer_in_seq_no);
        self.negotiated_layer = new_state["layer"]
            .as_i64()
            .unwrap_or(self.negotiated_layer)
            .max(announced);
        self.wrapper_layer = new_state["wrapper_layer"]
            .as_i64()
            .unwrap_or(self.wrapper_layer);
        self.gap_requested = new_state["gap_requested"] == true;
        self.gap_end = new_state["gap_end"].as_i64();
        self.resend_due = sequence_span(&new_state["resend_due"]);
        if let Some(span) = sequence_span(&seq_result["resend"]) {
            self.resend_request = Some(span);
        } else if !self.gap_requested {
            self.resend_request = None;
        }
        if let Some(pending) = seq_result["pending"].as_array() {
            self.pending_frames = pending.clone();
        }
        self.pending_frame_keys.insert(sequence, fingerprint);
        self.pending_frame_keys
            .retain(|sequence, _| *sequence >= self.in_seq_no);
        self.rekey.incoming_key_cutover = key_cutover;

        if let Some(action) = immediate {
            self.immediate_actions.push(action);
        }
        if key_slot == 1 {
            self.adopt_rekey_key(KeySlot(*key_bytes), true)?;
            self.rekey.pending_noop = true;
        } else if key_slot == 0 && self.rekey.previous.is_some() {
            self.rekey.confirmed = true;
        }
        if key_slot != 2 {
            self.rekey.messages = self.rekey.messages.saturating_add(1);
        }
        for message in &accepted {
            if let Some(ttl_raw) = message
                .action
                .as_ref()
                .filter(|act| act["id"] == 0xa173_3aec_u32)
                .and_then(|act| act["fields"]["ttl_seconds"].as_i64())
                .and_then(|v| i32::try_from(v).ok())
                .filter(|v| *v >= 0)
            {
                self.ttl = ttl_raw;
            }
        }
        self.retire_previous_key_if_settled();
        Ok(accepted)
    }
}
