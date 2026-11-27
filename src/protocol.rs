use aes::{
    Aes256,
    cipher::{Array, BlockCipherEncrypt, KeyInit},
};
use serde_json::{Value, json};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::errors::SecretChatError;

const INITIAL_REMOTE_LAYER: i64 = 46;
const SHARED_KEY_LEN: usize = 256;
const MIN_PADDING_LEN: usize = 12;
const WRAPPER_CONSTRUCTOR_ID: u32 = 0x1be3_1789;
const DECRYPTED_MESSAGE_73_ID: u32 = 0x91cc_4674;

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
struct KeySlot([u8; SHARED_KEY_LEN]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedMessage {
    pub layer: i64,
    pub in_seq_no: i64,
    pub out_seq_no: i64,
    pub message_type: String,
    pub text: Option<String>,
    pub raw_inner_bytes: Vec<u8>,
}

/// Core MTProto 2.0 secret-chat state aligned with `specs/019-rust-anything-full-rewrite/data-model.md`.
pub struct SecretChat {
    chat_id: i64,
    access_hash: i64,
    peer_user_id: i64,
    caller_auth_id: String,
    role: ChatRole,
    state: ChatState,
    negotiated_layer: i64,
    in_seq_no: i64,
    out_seq_no: i64,
    peer_in_seq_no: i64,
    key_lifecycle: KeyLifecycleState,
    active_key: Option<KeySlot>,
    pending_frames: Vec<Value>,
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
            in_seq_no: 0,
            out_seq_no: 0,
            peer_in_seq_no: 0,
            key_lifecycle: KeyLifecycleState::Unset,
            active_key: None,
            pending_frames: Vec::new(),
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

    pub fn in_seq_no(&self) -> i64 {
        self.in_seq_no
    }

    pub fn out_seq_no(&self) -> i64 {
        self.out_seq_no
    }

    pub fn set_out_seq_no(&mut self, value: i64) {
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
        Ok(fingerprint)
    }

    pub fn close(&mut self) {
        self.active_key = None;
        self.key_lifecycle = KeyLifecycleState::Zeroized;
        self.state = ChatState::Closed;
        self.pending_frames.clear();
    }

    pub(crate) fn export_key_for_internal_frame(&self) -> Option<[u8; SHARED_KEY_LEN]> {
        self.active_key.as_ref().map(|slot| slot.0)
    }

    pub fn receive_encrypted_frame(
        &mut self,
        frame: &[u8],
    ) -> Result<AcceptedMessage, SecretChatError> {
        if self.state == ChatState::Closed {
            return Err(SecretChatError::ChatClosed {
                chat_id: self.chat_id,
                reason: "chat is closed".into(),
            });
        }
        let key_bytes = match &self.active_key {
            Some(slot) => slot.0,
            None => {
                return Err(SecretChatError::ChatNotReady {
                    chat_id: self.chat_id,
                    state: format!("{:?}", self.state).to_lowercase(),
                });
            }
        };

        let decrypted_body = decrypt_frame(&key_bytes, frame, self.in_x())?;
        let unwrapped = unwrap_frame(&decrypted_body, Some(self.chat_id))?;

        let wrapper_layer =
            unwrapped["layer"]
                .as_i64()
                .ok_or_else(|| SecretChatError::MessageRejected {
                    chat_id: Some(self.chat_id),
                    reason: "missing wrapper layer".into(),
                })?;
        let wire_in_seq =
            unwrapped["in_seq_no"]
                .as_i64()
                .ok_or_else(|| SecretChatError::MessageRejected {
                    chat_id: Some(self.chat_id),
                    reason: "missing wrapper in_seq_no".into(),
                })?;
        let wire_out_seq =
            unwrapped["out_seq_no"]
                .as_i64()
                .ok_or_else(|| SecretChatError::MessageRejected {
                    chat_id: Some(self.chat_id),
                    reason: "missing wrapper out_seq_no".into(),
                })?;
        let inner_bytes = extract_tagged_bytes(&unwrapped["message"]).ok_or_else(|| {
            SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id),
                reason: "missing inner message bytes".into(),
            }
        })?;

        let state_json = json!({
            "id": self.chat_id,
            "is_outbound": self.is_outbound(),
            "in_seq_no": self.in_seq_no,
            "out_seq_no": self.out_seq_no,
            "peer_in_seq_no": self.peer_in_seq_no,
            "layer": self.negotiated_layer,
            "wrapper_layer": self.negotiated_layer,
            "gap_requested": false,
            "gap_end": Value::Null,
            "resend_due": Value::Null,
        });
        let frame_json = json!({
            "body": hex::encode(&decrypted_body),
            "file": Value::Null,
        });

        let seq_result =
            crate::ported::sequence::accept(state_json, frame_json, self.pending_frames.clone())
                .map_err(|err| SecretChatError::MessageRejected {
                    chat_id: Some(self.chat_id),
                    reason: sanitize_error_reason(&err),
                })?;

        let ready = seq_result["ready"].as_array().cloned().unwrap_or_default();
        if ready.is_empty() {
            return Err(SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id),
                reason: "duplicate or out-of-order frame was not accepted".into(),
            });
        }

        let new_state = &seq_result["state"];
        let decoded_inner = crate::ported::schema_secret_tl::decode_object(
            hex::encode(&inner_bytes),
            String::new(),
            0,
        )
        .map_err(|err| SecretChatError::MessageRejected {
            chat_id: Some(self.chat_id),
            reason: sanitize_error_reason(&err),
        })?;

        let inner_obj = &decoded_inner["object"];
        let message_type = constructor_name(inner_obj["id"].as_u64().unwrap_or(0));
        let text = inner_obj["fields"]["message"].as_str().map(str::to_owned);

        // Commit state updates only after every cryptographic, schema, and sequence check succeeded.
        self.in_seq_no = new_state["in_seq_no"].as_i64().unwrap_or(self.in_seq_no);
        self.peer_in_seq_no = new_state["peer_in_seq_no"]
            .as_i64()
            .unwrap_or(self.peer_in_seq_no);
        self.negotiated_layer = raise_remote_layer(self.negotiated_layer, wrapper_layer)?;
        if let Some(pending) = seq_result["pending"].as_array() {
            self.pending_frames = pending.clone();
        }

        Ok(AcceptedMessage {
            layer: wrapper_layer,
            in_seq_no: wire_in_seq,
            out_seq_no: wire_out_seq,
            message_type,
            text,
            raw_inner_bytes: inner_bytes,
        })
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
    crate::ported::crypto::key_fingerprint(shared_key.to_vec()).map_err(|err| {
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
    let bytes = crate::ported::crypto::compute_msg_key(shared_key.to_vec(), payload.to_vec(), x)
        .map_err(|err| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        })?;
    bytes
        .try_into()
        .map_err(|_| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "derived message key length mismatch".into(),
        })
}

pub fn derive_keys(
    shared_key: &[u8],
    msg_key: &[u8],
    x: i64,
) -> Result<([u8; 32], [u8; 32]), SecretChatError> {
    let val = crate::ported::crypto::derive_keys(shared_key.to_vec(), msg_key.to_vec(), x)
        .map_err(|err| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        })?;
    let pair = val["$tuple"]
        .as_array()
        .ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "malformed key derivation tuple".into(),
        })?;
    let k = extract_tagged_bytes(&pair[0]).ok_or_else(|| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: "malformed derived AES key".into(),
    })?;
    let iv = extract_tagged_bytes(&pair[1]).ok_or_else(|| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: "malformed derived AES IV".into(),
    })?;
    let key_arr: [u8; 32] = k
        .try_into()
        .map_err(|_| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "derived AES key must be 32 bytes".into(),
        })?;
    let iv_arr: [u8; 32] = iv
        .try_into()
        .map_err(|_| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "derived AES IV must be 32 bytes".into(),
        })?;
    Ok((key_arr, iv_arr))
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
    let fp = key_fingerprint(shared_key)?;
    let mut padded = pad_payload(body)?;
    let msg_key = compute_msg_key(shared_key, &padded, x)?;
    let (aes_key, aes_iv) = derive_keys(shared_key, &msg_key, x)?;
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
    let res = crate::ported::crypto::decrypt_frame(shared_key.to_vec(), frame.to_vec(), x, None)
        .map_err(|err| SecretChatError::MessageRejected {
            chat_id: None,
            reason: sanitize_error_reason(&err),
        })?;
    if let Some(reason) = res["reason"].as_str() {
        return Err(SecretChatError::MessageRejected {
            chat_id: None,
            reason: reason.to_owned(),
        });
    }
    extract_tagged_bytes(&res["body"]).ok_or_else(|| SecretChatError::MessageRejected {
        chat_id: None,
        reason: "decrypted frame body missing".into(),
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

pub fn verify_dh_fingerprint(
    chat_id: i64,
    key: &[u8],
    claimed: i64,
) -> Result<(), SecretChatError> {
    let val = crate::ported::handshake::verify_fingerprint(chat_id, key.to_vec(), claimed)
        .map_err(|err| SecretChatError::ParameterRejected {
            chat_id: Some(chat_id),
            reason: sanitize_error_reason(&err),
        })?;
    if let Some(reason) = val["reason"].as_str() {
        return Err(SecretChatError::ParameterRejected {
            chat_id: Some(chat_id),
            reason: reason.to_owned(),
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

fn sanitize_error_reason(raw: &str) -> String {
    let trimmed = raw.split_once(": ").map_or(raw, |(_, rest)| rest).trim();
    if trimmed.is_empty() {
        "protocol check failed".to_owned()
    } else {
        trimmed.to_owned()
    }
}
