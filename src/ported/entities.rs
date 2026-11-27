//! The Rust port of `rust/adapters/entities_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `to_secret` (`rust/adapters/entities_adapter.py:12`).
pub fn to_secret(
    items: serde_json::Value,
    layer: i64,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    use serde_json::{Map, Value, json};
    let Value::Array(items) = items else {
        let iterable = items.is_object() || items.is_string() || items.is_null();
        if iterable || items.as_bool() == Some(false) || items.as_f64() == Some(0.0) {
            return Ok(None);
        }
        let kind = if items.is_boolean() {
            "bool"
        } else if items.is_f64() {
            "float"
        } else {
            "int"
        };
        return Err(format!("TypeError: '{kind}' object is not iterable"));
    };
    let mut kept = Vec::new();
    for item in items {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(kind) = name.strip_prefix("MessageEntity") else {
            continue;
        };
        let (minimum, extra) = match kind {
            "Underline" | "Strike" | "Blockquote" => (101, None),
            "Spoiler" => (144, None),
            "CustomEmoji" => (144, Some("document_id")),
            "Pre" => (0, Some("language")),
            "TextUrl" => (0, Some("url")),
            "Unknown" | "Mention" | "Hashtag" | "BotCommand" | "Url" | "Email" | "Bold"
            | "Italic" | "Code" => (0, None),
            _ => continue,
        };
        if minimum > layer {
            continue;
        }
        let input = &item["fields"];
        let empty = matches!(input, Value::Null | Value::Bool(false))
            || input.as_array().is_some_and(Vec::is_empty)
            || input.as_str().is_some_and(str::is_empty)
            || input.as_i64() == Some(0);
        if !input.is_object() && !empty {
            continue;
        }
        let fields: Map<_, _> = ["offset", "length"]
            .into_iter()
            .chain(extra)
            .map(|field| (field.into(), input[field].clone()))
            .collect();
        kept.push(json!({"name":name,"fields":fields}));
    }
    Ok((!kept.is_empty()).then_some(kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_by_layer_and_preserves_declared_fields() -> Result<(), String> {
        let items = vec![
            json!({"name":"MessageEntitySpoiler","fields":{"length":2}}),
            json!({"name":"MessageEntityTextUrl","fields":{"offset":1,"length":2,"url":"https://example.invalid","ignored":42}}),
            json!({"name":"MessageEntityMentionName","fields":{"offset":1}}),
        ];
        assert_eq!(
            to_secret(json!(items), 143)?,
            Some(vec![
                json!({"name":"MessageEntityTextUrl","fields":{"offset":1,"length":2,"url":"https://example.invalid"}})
            ])
        );
        assert_eq!(to_secret(json!(items), 144)?.map(|v| v.len()), Some(2));
        assert_eq!(to_secret(json!([]), 144)?, None);
        assert_eq!(to_secret(json!({"a":"b"}), 144)?, None);
        Ok(())
    }
}
