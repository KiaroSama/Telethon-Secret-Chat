//! Safe exception descriptions and their migration representation.

use serde_json::json;
use std::fmt::Write;

fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for ch in text.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c == quote => {
                result.push('\\');
                result.push(c);
            }
            c if c != ' '
                && (c.is_control()
                    || c.is_whitespace()
                    || matches!(c, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')) =>
            {
                let n = u32::from(c);
                if n <= 0xff {
                    let _ = write!(result, "\\x{n:02x}");
                } else if n <= 0xffff {
                    let _ = write!(result, "\\u{n:04x}");
                } else {
                    let _ = write!(result, "\\U{n:08x}");
                }
            }
            c => result.push(c),
        }
    }
    result.push(quote);
    result
}

pub fn describe_error(
    kind: String,
    text: String,
    chat_id: Option<i64>,
    first: Option<i64>,
    last: Option<i64>,
    retained_from: Option<i64>,
    fatal: bool,
) -> Result<Option<serde_json::Value>, String> {
    let mut fields = json!({
        "chat_id": if matches!(kind.as_str(), "StorageRequired" | "ManagerStopping") {None} else {chat_id},
        "fatal": fatal && matches!(kind.as_str(), "SecretChatError" | "MessageRejected" | "ResendUnsatisfiable"),
        "reason": "refused",
    });
    let display_int =
        |value: Option<i64>| value.map_or_else(|| String::from("None"), |n| n.to_string());
    let message = match kind.as_str() {
        "SecretChatError" => text,
        "ParameterRejected" | "MessageRejected" | "ChatClosed" | "StoreCorrupt" => {
            let prefix = match kind.as_str() {
                "ParameterRejected" => "refused the key exchange",
                "MessageRejected" => "rejected a received message",
                "ChatClosed" => "the chat is closed",
                _ => "the secret-chat store is corrupt",
            };
            fields["reason"] = json!(text);
            format!("{prefix}: {text}")
        }
        "ChatNotReady" => {
            fields["state"] = json!(text);
            format!("the chat is {}; this operation needs an established chat", python_repr(&text))
        }
        "StorageRequired" => "a storage backend is required: secret-chat keys must be persisted somewhere the application chose. There is no default, deliberately - see telethon_secret_chat.storage for the two shipped backends".into(),
        "LayerUnsupported" => {
            fields["peer_layer"] = json!(first);
            format!("the peer announced layer {}; this package requires 73 or higher, below which the protocol is MTProto 1.0 and out of scope", display_int(first))
        }
        "ResendUnsatisfiable" => {
            fields["reason"] = json!("a resend request could not be satisfied");
            fields["requested"] = json!([first, last]);
            fields["retained_from"] = json!(retained_from);
            format!("cannot resend messages {}-{}: this side retains from {}. The protocol requires the chat to end rather than answer a resend it cannot satisfy", display_int(first), display_int(last), display_int(retained_from))
        }
        "UnknownChat" => "no secret chat with this id in this manager".into(),
        "ManagerStopping" => "the secret-chat manager is stopping".into(),
        "SendPending" => {
            fields["reason"] = json!("a message is stored and will be sent again");
            fields["random_id"] = json!(first);
            fields["cause"] = json!(text);
            format!("a message is stored and will be sent again (the transport failed with {text})")
        }
        _ => return Ok(None),
    };
    let message = match fields["chat_id"].as_i64() {
        Some(chat) => format!("[chat {chat}] {message}"),
        None => message,
    };
    Ok(Some(json!({
        "kind": kind,
        "message": message,
        "repr": format!("{kind}({})", python_repr(&message)),
        "fields": fields,
        "is_secret_chat_error": true,
        "is_key_error": kind == "UnknownChat",
        "is_runtime_error": kind == "ManagerStopping",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_keep_their_safe_fields_and_legacy_bases() -> Result<(), String> {
        for (kind, text, expected) in [
            ("SecretChatError", "refused", "refused"),
            (
                "ParameterRejected",
                "invalid peer value",
                "refused the key exchange",
            ),
            (
                "MessageRejected",
                "invalid length",
                "rejected a received message",
            ),
            ("ChatNotReady", "requested", "needs an established chat"),
            ("ChatClosed", "peer discarded it", "the chat is closed"),
            ("StorageRequired", "", "a storage backend is required"),
            ("LayerUnsupported", "", "the peer announced layer 8"),
            ("ResendUnsatisfiable", "", "cannot resend messages 8-9"),
            (
                "StoreCorrupt",
                "invalid record",
                "the secret-chat store is corrupt",
            ),
            ("UnknownChat", "", "no secret chat with this id"),
            ("ManagerStopping", "", "the secret-chat manager is stopping"),
            (
                "SendPending",
                "OSError",
                "the transport failed with OSError",
            ),
        ] {
            let e = describe_error(
                kind.into(),
                text.into(),
                Some(42),
                Some(8),
                Some(9),
                Some(7),
                true,
            )?
            .ok_or("Missing error contract")?;
            let message = e["message"].as_str().ok_or("Missing safe message")?;
            let repr = e["repr"].as_str().ok_or("Missing safe repr")?;
            assert!(message.contains(expected));
            assert!(repr.starts_with(kind));
            assert!(repr.contains(expected));
            assert_eq!(e["is_secret_chat_error"], true);
            assert_eq!(e["is_key_error"], kind == "UnknownChat");
            assert_eq!(e["is_runtime_error"], kind == "ManagerStopping");
            let carries_context = !matches!(kind, "StorageRequired" | "ManagerStopping");
            assert_eq!(
                e["fields"]["chat_id"].as_i64(),
                carries_context.then_some(42)
            );
            assert_eq!(
                e["fields"]["fatal"],
                matches!(
                    kind,
                    "SecretChatError" | "MessageRejected" | "ResendUnsatisfiable"
                )
            );
            assert!(e["fields"]["reason"].is_string());
            for rendered in [message, repr] {
                assert!(!rendered.contains("synthetic private message"));
                assert!(!rendered.contains(&"0fad".repeat(64)));
                assert!(
                    !rendered
                        .split(|ch: char| !ch.is_ascii_alphanumeric())
                        .any(|word| word.len() >= 40)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn unknown_kind_has_no_exception_contract() -> Result<(), String> {
        assert_eq!(
            describe_error(
                "not-a-constructor".into(),
                String::new(),
                None,
                None,
                None,
                None,
                false
            )?,
            None
        );
        Ok(())
    }
}
