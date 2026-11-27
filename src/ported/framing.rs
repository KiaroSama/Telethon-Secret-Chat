//! The Rust port of `rust/adapters/framing_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

// Import only the verified core; linking both PyO3 bridges would duplicate PyInit__native.
use super::schema_secret_tl as schema_core;
use schema_core::{decode_object, encode_object};
use serde_json::{Value, json};
use std::fmt::Write;

const WRAPPER: u64 = 0x1be31789;
const INITIAL_REMOTE_LAYER: i64 = 46;
const MIN_LAYER: i64 = 73;
const MAX_LAYER: i64 = 144;
const RANDOM_BYTES_LENGTH: usize = 31;

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or_else(|| "ValueError: malformed TL bytes".into())
        })
        .collect()
}

fn header_parses(body: &[u8]) -> bool {
    if !body.starts_with(&0x1be31789u32.to_le_bytes()) {
        return false;
    }
    let Some(&marker) = body.get(4) else {
        return false;
    };
    let (prefix, size) = if marker == 254 {
        let Some(n) = body.get(5..8) else {
            return false;
        };
        (4, u32::from_le_bytes([n[0], n[1], n[2], 0]) as usize)
    } else {
        (1, usize::from(marker))
    };
    body.len() > 16 + (prefix + size).next_multiple_of(4)
}

fn refusal(kind: &str, reason: &str, detail: &str, chat_id: Option<i64>, fatal: bool) -> Value {
    let chat = chat_id.map_or_else(String::new, |n| format!("[chat {n}] "));
    json!({"error_type":kind,"message":format!("{chat}{detail}"),"reason":reason,"fatal":fatal})
}

fn rejected(reason: &str, chat_id: Option<i64>, fatal: bool) -> Value {
    refusal(
        "MessageRejected",
        reason,
        &format!("rejected a received message: {reason}"),
        chat_id,
        fatal,
    )
}

fn byte_value(bytes: Vec<u8>) -> Value {
    json!({"$bytes":bytes})
}

/// Port of `wrap` (`rust/adapters/framing_adapter.py:17`).
pub fn wrap(
    message: Vec<u8>,
    layer: i64,
    in_seq_no: i64,
    out_seq_no: i64,
) -> Result<serde_json::Value, String> {
    let Ok(parsed) = decode_object(encode_hex(&message), "DecryptedMessage".into(), 0) else {
        return Ok(json!({"invalid_message":true}));
    };
    if parsed["position"].as_u64() != Some(message.len() as u64) {
        return Ok(json!({"invalid_message":true}));
    }
    let mut random = [0; RANDOM_BYTES_LENGTH];
    getrandom::fill(&mut random).map_err(|_| "RuntimeError: OS randomness unavailable")?;
    let encoded = parsed["encoded"]
        .as_str()
        .ok_or("Missing TL message encoding")?;
    Ok(
        json!({"random_bytes_length":random.len(),"layer":layer,"in_seq_no":in_seq_no,"out_seq_no":out_seq_no,"message":byte_value(decode_hex(encoded)?)}),
    )
}

/// Port of `unwrap` (`rust/adapters/framing_adapter.py:29`).
///
/// `chat_id` defaults to `None` in the source.
pub fn unwrap(body: Vec<u8>, chat_id: Option<i64>) -> Result<serde_json::Value, String> {
    let parsed = match decode_object(encode_hex(&body), String::new(), 0) {
        Ok(v) if v["position"].as_u64() == Some(body.len() as u64) => v,
        Err(error) if error.starts_with("ValueError: UnknownConstructor:") => {
            return Ok(rejected(
                "the message names a constructor this schema does not define",
                chat_id,
                header_parses(&body),
            ));
        }
        _ => {
            return Ok(rejected(
                "the message could not be parsed",
                chat_id,
                header_parses(&body),
            ));
        }
    };
    let wrapper = &parsed["object"];
    if wrapper["id"].as_u64() != Some(WRAPPER) {
        return Ok(rejected(
            "the message is not wrapped in decryptedMessageLayer",
            chat_id,
            false,
        ));
    }
    let fields = &wrapper["fields"];
    if fields["layer"]
        .as_i64()
        .is_some_and(|layer| layer < INITIAL_REMOTE_LAYER)
    {
        return Ok(rejected(
            "the message announces a layer below the documented minimum",
            chat_id,
            false,
        ));
    }
    let inner = &fields["message"];
    if !matches!(
        inner["id"].as_u64(),
        Some(0x91cc4674 | 0x1f814f1f | 0x204d3878 | 0x36b091de | 0x73164160 | 0xaa48327d)
    ) {
        return Ok(rejected(
            "the wrapper does not contain a message",
            chat_id,
            false,
        ));
    }
    let random = fields["random_bytes"]["hex"]
        .as_str()
        .ok_or("Missing wrapper bytes")?;
    let random = decode_hex(random)?;
    if random.len() < 15 {
        return Ok(rejected(
            "the wrapper contains fewer than 15 random bytes",
            chat_id,
            false,
        ));
    }
    Ok(
        json!({"random_bytes":byte_value(random),"layer":fields["layer"],"in_seq_no":fields["in_seq_no"],"out_seq_no":fields["out_seq_no"],"message":byte_value(decode_hex(&encode_object(inner.clone())?)?)}),
    )
}

/// Port of `raise_remote_layer` (`rust/adapters/framing_adapter.py:37`).
pub fn raise_remote_layer(stored: i64, announced: i64) -> Result<i64, String> {
    Ok(stored.max(announced))
}

/// Port of `outgoing_layer` (`rust/adapters/framing_adapter.py:41`).
pub fn outgoing_layer(remote_layer: i64) -> Result<i64, String> {
    Ok(remote_layer.clamp(MIN_LAYER, MAX_LAYER))
}

/// Port of `require_supported_layer` (`rust/adapters/framing_adapter.py:45`).
pub fn require_supported_layer(chat_id: i64, peer_layer: i64) -> Result<serde_json::Value, String> {
    if peer_layer >= MIN_LAYER {
        return Ok(json!({"ok":null}));
    }
    let message = format!(
        "the peer announced layer {peer_layer}; this package requires 73 or higher, below which the protocol is MTProto 1.0 and out of scope"
    );
    Ok(refusal(
        "LayerUnsupported",
        "refused",
        &message,
        Some(chat_id),
        false,
    ))
}

/// Port of `transform_in_seq_no` (`rust/adapters/framing_adapter.py:52`).
pub fn transform_in_seq_no(raw: i64, is_outbound: bool) -> Result<String, String> {
    Ok((i128::from(raw) * 2 + i128::from(!is_outbound)).to_string())
}

/// Port of `transform_out_seq_no` (`rust/adapters/framing_adapter.py:56`).
pub fn transform_out_seq_no(raw: i64, is_outbound: bool) -> Result<String, String> {
    Ok((i128::from(raw) * 2 + i128::from(is_outbound)).to_string())
}

/// Port of `raw_seq_no` (`rust/adapters/framing_adapter.py:60`).
pub fn raw_seq_no(wire: i64) -> Result<i64, String> {
    Ok(wire.div_euclid(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_layer_limits_before_acceptance() -> Result<(), String> {
        assert_eq!(outgoing_layer(46)?, 73);
        assert_eq!(outgoing_layer(145)?, 144);
        assert_eq!(
            require_supported_layer(42, 72)?["error_type"],
            "LayerUnsupported"
        );
        assert_eq!(unwrap(vec![0; 3], None)?["fatal"], false);
        assert_eq!(transform_out_seq_no(2, true)?, "5");
        assert_eq!(raw_seq_no(-3)?, -2);
        Ok(())
    }
}
