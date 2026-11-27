use aes::{
    Aes256,
    cipher::{Array, BlockCipherEncrypt, KeyInit},
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    Initiator,
    Acceptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatState {
    Requested,
    Offered,
    Ready,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub(crate) key_lifecycle: KeyLifecycleState,
    pub(crate) active_key: Option<KeySlot>,
    pub(crate) rekey: crate::ported::rekey::RuntimeRekey,
    pending_frames: Vec<Value>,
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
            key_lifecycle: KeyLifecycleState::Unset,
            active_key: None,
            rekey: crate::ported::rekey::RuntimeRekey::default(),
            pending_frames: Vec::new(),
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
        self.ready_messages.clear();
        self.immediate_actions.clear();
        self.gap_requested = false;
        self.gap_end = None;
        self.resend_due = None;
        self.resend_request = None;
    }

    pub(crate) fn export_key_for_internal_frame(&self) -> Option<[u8; SHARED_KEY_LEN]> {
        self.active_key.as_ref().map(|slot| slot.0)
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
        let named = frame
            .get(..8)
            .ok_or_else(|| SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id),
                reason: "the frame is too short to hold a message".into(),
            })?;
        let selected = [
            self.active_key.as_ref(),
            self.rekey.pending.as_ref(),
            self.rekey.previous.as_ref(),
        ]
        .into_iter()
        .enumerate()
        .find_map(|(index, slot)| {
            let slot = slot?;
            let fingerprint = key_fingerprint(&slot.0).ok()?;
            bool::from(named.ct_eq(&fingerprint.to_le_bytes()))
                .then(|| (index, Zeroizing::new(slot.0)))
        })
        .ok_or_else(|| SecretChatError::MessageRejected {
            chat_id: Some(self.chat_id),
            reason: "the key fingerprint does not name a key this chat holds".into(),
        })?;
        let (key_slot, key_bytes) = selected;

        let chat_id = self.chat_id;
        let rejected = |reason| SecretChatError::MessageRejected {
            chat_id: Some(chat_id),
            reason,
        };
        let decrypted_body = Zeroizing::new(decrypt_frame(&*key_bytes, frame, self.in_x())?);
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
        self.retire_previous_key_if_settled();
        Ok(accepted)
    }
}

pub fn generate_secret_bytes() -> Result<[u8; SHARED_KEY_LEN], SecretChatError> {
    let mut bytes = [0u8; SHARED_KEY_LEN];
    getrandom::fill(&mut bytes).map_err(|err| SecretChatError::EntropyUnavailable {
        reason: err.to_string(),
    })?;
    Ok(bytes)
}

pub fn key_fingerprint(shared_key: &[u8]) -> Result<i64, SecretChatError> {
    crate::ported::crypto::key_fingerprint_native(shared_key).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn compute_msg_key(
    shared_key: &[u8],
    payload: &[u8],
    x: i64,
) -> Result<[u8; 16], SecretChatError> {
    crate::ported::crypto::compute_msg_key_native(shared_key, payload, x).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn derive_keys(
    shared_key: &[u8],
    msg_key: &[u8],
    x: i64,
) -> Result<([u8; 32], [u8; 32]), SecretChatError> {
    crate::ported::crypto::derive_keys_native(shared_key, msg_key, x).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn pad_payload(body: &[u8]) -> Result<Vec<u8>, SecretChatError> {
    let mut payload = Vec::with_capacity(4 + body.len() + MIN_PADDING_LEN + 16);
    let len_u32 = u32::try_from(body.len()).map_err(|_| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: "payload exceeds maximum TL length".into(),
    })?;
    payload.extend_from_slice(&len_u32.to_le_bytes());
    payload.extend_from_slice(body);
    let mut pad_len = MIN_PADDING_LEN;
    while !(payload.len() + pad_len).is_multiple_of(16) {
        pad_len += 1;
    }
    let mut padding = vec![0u8; pad_len];
    getrandom::fill(&mut padding).map_err(|err| SecretChatError::EntropyUnavailable {
        reason: err.to_string(),
    })?;
    payload.extend_from_slice(&padding);
    Ok(payload)
}

pub fn encrypt_frame(shared_key: &[u8], body: &[u8], x: i64) -> Result<Vec<u8>, SecretChatError> {
    if !body.len().is_multiple_of(4) {
        return Err(SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "serialized TL body length must be divisible by four".into(),
        });
    }
    let fp = key_fingerprint(shared_key)?;
    let mut padded = Zeroizing::new(pad_payload(body)?);
    let msg_key = compute_msg_key(shared_key, &padded, x)?;
    let (aes_key, aes_iv) = derive_keys(shared_key, &msg_key, x)?;
    let aes_key = Zeroizing::new(aes_key);
    let aes_iv = Zeroizing::new(aes_iv);
    encrypt_ige_in_place(&mut padded, &aes_key, &aes_iv);
    let mut frame = Vec::with_capacity(8 + 16 + padded.len());
    frame.extend_from_slice(&fp.to_le_bytes());
    frame.extend_from_slice(&msg_key);
    frame.extend_from_slice(&padded);
    Ok(frame)
}

fn encrypt_ige_in_place(payload: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
    const BLOCK: usize = 16;
    let cipher = Aes256::new(&Array::from(*key));
    let mut previous_cipher = [0u8; BLOCK];
    let mut previous_plain = [0u8; BLOCK];
    previous_cipher.copy_from_slice(&iv[..BLOCK]);
    previous_plain.copy_from_slice(&iv[BLOCK..]);
    for chunk in payload.as_chunks_mut::<BLOCK>().0 {
        let mut input = [0u8; BLOCK];
        input.copy_from_slice(chunk);
        let mut block = Array::from(std::array::from_fn(|i| input[i] ^ previous_cipher[i]));
        cipher.encrypt_block(&mut block);
        for i in 0..BLOCK {
            chunk[i] = block[i] ^ previous_plain[i];
        }
        previous_plain = input;
        previous_cipher.copy_from_slice(chunk);
    }
}

pub fn decrypt_frame(shared_key: &[u8], frame: &[u8], x: i64) -> Result<Vec<u8>, SecretChatError> {
    crate::ported::crypto::decrypt_frame_native(shared_key, frame, x).map_err(|err| {
        SecretChatError::MessageRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn wrap_frame(
    inner: &Value,
    layer: i64,
    in_seq_no: i64,
    out_seq_no: i64,
) -> Result<Vec<u8>, SecretChatError> {
    let mut random_bytes = [0u8; 31];
    getrandom::fill(&mut random_bytes).map_err(|err| SecretChatError::EntropyUnavailable {
        reason: err.to_string(),
    })?;

    let message_obj = json!({
        "id": DECRYPTED_MESSAGE_73_ID,
        "fields": {
            "random_id": inner["random_id"].as_i64().unwrap_or(1),
            "silent": inner.get("silent").cloned().unwrap_or(Value::Bool(false)),
            "no_webpage": inner.get("no_webpage").cloned().unwrap_or(Value::Bool(false)),
            "ttl": inner["ttl"].as_i64().unwrap_or(0),
            "message": inner["message"].as_str().unwrap_or(""),
            "media": inner.get("media").cloned().unwrap_or(Value::Null),
            "entities": inner.get("entities").cloned().unwrap_or_else(|| json!([])),
            "via_bot_name": inner.get("via_bot_name").cloned().unwrap_or(Value::Null),
            "reply_to_random_id": inner.get("reply_to_random_id").cloned().unwrap_or(Value::Null),
            "grouped_id": inner.get("grouped_id").cloned().unwrap_or(Value::Null),
        }
    });

    let wrapper = json!({
        "id": WRAPPER_CONSTRUCTOR_ID,
        "fields": {
            "random_bytes": { "hex": hex::encode(random_bytes) },
            "layer": layer,
            "in_seq_no": in_seq_no,
            "out_seq_no": out_seq_no,
            "message": message_obj,
        }
    });

    let encoded_hex = crate::ported::schema_secret_tl::encode_object(wrapper).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })?;
    hex::decode(encoded_hex).map_err(|_| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: "encoded wrapper hex invalid".into(),
    })
}

pub fn unwrap_frame(body: &[u8], chat_id: Option<i64>) -> Result<Value, SecretChatError> {
    let res = crate::ported::framing::unwrap(body.to_vec(), chat_id).map_err(|err| {
        SecretChatError::MessageRejected {
            chat_id,
            reason: sanitize_error_reason(&err),
        }
    })?;
    if res.get("error_type").is_some() {
        let reason = res["reason"]
            .as_str()
            .unwrap_or("the message could not be parsed")
            .to_owned();
        return Err(SecretChatError::MessageRejected { chat_id, reason });
    }
    Ok(res)
}

pub fn outgoing_layer(remote_layer: i64) -> Result<i64, SecretChatError> {
    crate::ported::framing::outgoing_layer(remote_layer).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn raise_remote_layer(stored: i64, announced: i64) -> Result<i64, SecretChatError> {
    crate::ported::framing::raise_remote_layer(stored, announced).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn require_supported_layer(chat_id: i64, peer_layer: i64) -> Result<(), SecretChatError> {
    let res =
        crate::ported::framing::require_supported_layer(chat_id, peer_layer).map_err(|err| {
            SecretChatError::ParameterRejected {
                chat_id: Some(chat_id),
                reason: sanitize_error_reason(&err),
            }
        })?;
    if res.get("error_type").and_then(Value::as_str) == Some("LayerUnsupported") {
        return Err(SecretChatError::LayerUnsupported {
            chat_id,
            peer_layer,
        });
    }
    Ok(())
}

pub fn check_dh_config(
    g: i64,
    p_bytes: &[u8],
    chat_id: Option<i64>,
) -> Result<(), SecretChatError> {
    let g_bytes = g.to_be_bytes().to_vec();
    let chat_bytes = chat_id.map(|id| id.to_be_bytes().to_vec());
    let res = crate::ported::dh::check_config(g_bytes, to_signed_be_positive(p_bytes), chat_bytes)
        .map_err(|err| SecretChatError::ParameterRejected {
            chat_id,
            reason: sanitize_error_reason(&err),
        })?;
    if res.get("error_type").is_some() || res.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(SecretChatError::ParameterRejected {
            chat_id,
            reason: res["reason"]
                .as_str()
                .unwrap_or("DH parameters failed safety checks")
                .to_owned(),
        });
    }
    Ok(())
}

pub fn check_peer_value(
    value_bytes: &[u8],
    p_bytes: &[u8],
    chat_id: Option<i64>,
) -> Result<(), SecretChatError> {
    let chat_bytes = chat_id.map(|id| id.to_be_bytes().to_vec());
    let res = crate::ported::dh::check_peer_value(
        to_signed_be_positive(value_bytes),
        to_signed_be_positive(p_bytes),
        chat_bytes,
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id,
        reason: sanitize_error_reason(&err),
    })?;
    if res.get("error_type").is_some() || res.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(SecretChatError::ParameterRejected {
            chat_id,
            reason: res["reason"]
                .as_str()
                .unwrap_or("peer DH value is out of range")
                .to_owned(),
        });
    }
    Ok(())
}

pub fn key_visualization(data: &[u8]) -> Result<Value, SecretChatError> {
    crate::ported::visualization::key_visualization(data.to_vec()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn rekey_collision(mine: i64, theirs: i64) -> Result<String, SecretChatError> {
    crate::ported::rekey::collision(mine, theirs).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn dh_public_value(g: &[u8], secret: &[u8], p: &[u8]) -> Result<String, SecretChatError> {
    validate_dh_secret(secret, None)?;
    if p.len() == SHARED_KEY_LEN {
        let public = crate::ported::handshake::fixed_power(g, secret, p).map_err(|err| {
            SecretChatError::ParameterRejected {
                chat_id: None,
                reason: sanitize_error_reason(&err),
            }
        })?;
        return Ok(num_bigint::BigUint::from_bytes_be(&public).to_string());
    }
    crate::ported::handshake::public_value(
        to_signed_be_positive(g),
        to_signed_be_positive(secret),
        to_signed_be_positive(p),
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: sanitize_error_reason(&err),
    })
}

pub fn dh_shared_key(
    peer_value: &[u8],
    secret: &[u8],
    p: &[u8],
    chat_id: Option<i64>,
) -> Result<[u8; SHARED_KEY_LEN], SecretChatError> {
    validate_dh_secret(secret, chat_id)?;
    if p.len() == SHARED_KEY_LEN {
        check_peer_value(peer_value, p, chat_id)?;
        return crate::ported::handshake::fixed_power(peer_value, secret, p).map_err(|err| {
            SecretChatError::ParameterRejected {
                chat_id,
                reason: sanitize_error_reason(&err),
            }
        });
    }
    let val = crate::ported::handshake::shared_key(
        to_signed_be_positive(peer_value),
        to_signed_be_positive(secret),
        to_signed_be_positive(p),
        chat_id,
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id,
        reason: sanitize_error_reason(&err),
    })?;
    if let Some(reason) = val["reason"].as_str() {
        return Err(SecretChatError::ParameterRejected {
            chat_id,
            reason: reason.to_owned(),
        });
    }
    let bytes =
        extract_tagged_bytes(&val["key"]).ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id,
            reason: "derived DH shared key missing".into(),
        })?;
    bytes
        .try_into()
        .map_err(|_| SecretChatError::ParameterRejected {
            chat_id,
            reason: "derived DH shared key must be 256 bytes".into(),
        })
}

fn to_signed_be_positive(bytes: &[u8]) -> Vec<u8> {
    if bytes.first().is_some_and(|&b| b >= 0x80) {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x00);
        out.extend_from_slice(bytes);
        out
    } else {
        bytes.to_vec()
    }
}

fn validate_dh_secret(secret: &[u8], chat_id: Option<i64>) -> Result<(), SecretChatError> {
    if secret.is_empty() || secret.len() > SHARED_KEY_LEN || secret.iter().all(|&byte| byte == 0) {
        return Err(SecretChatError::ParameterRejected {
            chat_id,
            reason: "the DH secret must be a positive value of at most 2048 bits".into(),
        });
    }
    Ok(())
}

pub fn verify_dh_fingerprint(
    chat_id: i64,
    key: &[u8],
    claimed: i64,
) -> Result<(), SecretChatError> {
    let expected = key_fingerprint(key)?;
    if !bool::from(expected.to_le_bytes().ct_eq(&claimed.to_le_bytes())) {
        return Err(SecretChatError::ParameterRejected {
            chat_id: Some(chat_id),
            reason: "the key fingerprint the peer published does not match the key this side derived: the exchange did not produce a shared key and the chat must be discarded".into(),
        });
    }
    Ok(())
}

pub fn validate_protected_binding(binding: Value) -> Result<(), SecretChatError> {
    crate::ported::protected::validate_binding(binding).map_err(|err| {
        SecretChatError::StoreCorrupt {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

pub fn describe_protocol_error(
    kind: &str,
    chat_id: Option<i64>,
    reason: &str,
) -> Result<Option<Value>, SecretChatError> {
    crate::ported::errors::describe_error(
        kind.to_owned(),
        reason.to_owned(),
        chat_id,
        None,
        None,
        None,
        false,
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id,
        reason: sanitize_error_reason(&err),
    })
}

pub fn secret_entities_from_plain(
    items: Value,
    layer: i64,
) -> Result<Option<Vec<Value>>, SecretChatError> {
    crate::ported::entities::to_secret(items, layer).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        }
    })
}

fn extract_tagged_bytes(value: &Value) -> Option<Vec<u8>> {
    let arr = value.get("$bytes")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let byte = item.as_u64()?;
        out.push(u8::try_from(byte).ok()?);
    }
    Some(out)
}

fn constructor_name(id: u64) -> String {
    match id {
        0x91cc_4674 => "DecryptedMessage".to_owned(),
        0x36b0_91de => "DecryptedMessage45".to_owned(),
        0x204d_3878 => "DecryptedMessage17".to_owned(),
        0x1f81_4f1f => "DecryptedMessage8".to_owned(),
        0x7316_4160 => "DecryptedMessageService".to_owned(),
        0xaa48_327d => "DecryptedMessageService8".to_owned(),
        _ => "DecryptedMessage".to_owned(),
    }
}

fn sequence_span(value: &Value) -> Option<(i64, i64)> {
    Some((value.get(0)?.as_i64()?, value.get(1)?.as_i64()?))
}

fn accepted_record(record: &Value, chat_id: i64) -> Result<AcceptedMessage, SecretChatError> {
    let rejected = |reason| SecretChatError::MessageRejected {
        chat_id: Some(chat_id),
        reason,
    };
    let body = record["body"]
        .as_str()
        .ok_or_else(|| rejected("missing recovered message body".into()))?;
    let decoded = crate::ported::schema_secret_tl::decode_object(
        body.to_owned(),
        "DecryptedMessageLayer".into(),
        0,
    )
    .map_err(|err| rejected(sanitize_error_reason(&err)))?;
    let fields = &decoded["object"]["fields"];
    let number = |name| {
        fields[name]
            .as_i64()
            .ok_or_else(|| rejected("invalid recovered message counter".into()))
    };
    let inner = &fields["message"];
    let encoded = crate::ported::schema_secret_tl::encode_object(inner.clone())
        .map_err(|err| rejected(sanitize_error_reason(&err)))?;
    Ok(AcceptedMessage {
        layer: number("layer")?,
        in_seq_no: number("in_seq_no")?,
        out_seq_no: number("out_seq_no")?,
        message_type: constructor_name(inner["id"].as_u64().unwrap_or(0)),
        text: inner["fields"]["message"].as_str().map(str::to_owned),
        raw_inner_bytes: hex::decode(encoded)
            .map_err(|_| rejected("invalid recovered message encoding".into()))?,
        random_id: inner["fields"]["random_id"]
            .as_i64()
            .ok_or_else(|| rejected("missing recovered random identifier".into()))?,
        action: inner["fields"].get("action").cloned(),
    })
}

fn sanitize_error_reason(raw: &str) -> String {
    let trimmed = raw.split_once(": ").map_or(raw, |(_, rest)| rest).trim();
    if trimmed.is_empty() {
        "protocol check failed".to_owned()
    } else {
        trimmed.to_owned()
    }
}
