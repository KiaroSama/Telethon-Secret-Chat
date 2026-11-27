use aes::{
    Aes256,
    cipher::{Array, BlockCipherEncrypt, KeyInit},
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{
    AcceptedMessage, DECRYPTED_MESSAGE_73_ID, MIN_PADDING_LEN, SHARED_KEY_LEN,
    WRAPPER_CONSTRUCTOR_ID,
};
use crate::errors::SecretChatError;

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

pub(super) fn encrypt_ige_in_place(payload: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
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

pub(super) fn to_signed_be_positive(bytes: &[u8]) -> Vec<u8> {
    if bytes.first().is_some_and(|&b| b >= 0x80) {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x00);
        out.extend_from_slice(bytes);
        out
    } else {
        bytes.to_vec()
    }
}

pub(super) fn validate_dh_secret(
    secret: &[u8],
    chat_id: Option<i64>,
) -> Result<(), SecretChatError> {
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

pub(super) fn extract_tagged_bytes(value: &Value) -> Option<Vec<u8>> {
    let arr = value.get("$bytes")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let byte = item.as_u64()?;
        out.push(u8::try_from(byte).ok()?);
    }
    Some(out)
}

pub(super) fn constructor_name(id: u64) -> String {
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

pub(super) fn sequence_span(value: &Value) -> Option<(i64, i64)> {
    Some((value.get(0)?.as_i64()?, value.get(1)?.as_i64()?))
}

pub(super) fn accepted_record(
    record: &Value,
    chat_id: i64,
) -> Result<AcceptedMessage, SecretChatError> {
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

pub(super) fn sanitize_error_reason(raw: &str) -> String {
    let trimmed = raw.split_once(": ").map_or(raw, |(_, rest)| rest).trim();
    if trimmed.is_empty() {
        "protocol check failed".to_owned()
    } else {
        trimmed.to_owned()
    }
}
