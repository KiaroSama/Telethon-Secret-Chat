//! Byte serialization rules from Telegram Desktop's export writer (GPL-3.0).

use chrono::{Local, TimeZone};
use serde_json::Value;

fn escape(data: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = vec![b'"'];
    let mut i = 0;
    while i < data.len() {
        if let Some([0xe2, 0x80, last @ (0xa8 | 0xa9)]) = data.get(i..i + 3) {
            out.extend_from_slice(if *last == 0xa8 {
                b"\\u2028"
            } else {
                b"\\u2029"
            });
            out.extend([0x80, *last]);
            i += 3;
            continue;
        }
        let b = data[i];
        match b {
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'"' | b'\\' => out.extend([b'\\', b]),
            0..=31 => out.extend([
                b'\\',
                b'x',
                HEX[usize::from(b >> 4)],
                HEX[usize::from(b & 15)],
            ]),
            _ => out.push(b),
        }
        i += 1;
    }
    out.push(b'"');
    out
}
pub fn utf8(value: String) -> Result<Vec<u8>, String> {
    Ok(value.into_bytes())
}
pub fn number(value: Value) -> Result<Vec<u8>, String> {
    let text = if let Some(n) = value.as_i64() {
        n.to_string()
    } else if let Some(n) = value.as_u64() {
        n.to_string()
    } else if let Some(n) = value.as_f64() {
        format!("{n:.6}")
    } else if let Some(n) = value.as_bool() {
        i64::from(n).to_string()
    } else if let Some(n) = value.get("$int").and_then(Value::as_str) {
        n.to_owned()
    } else if let Some(n) = value
        .get("$num")
        .or_else(|| value.get("$float"))
        .and_then(Value::as_str)
    {
        match n {
            "NaN" | "nan" => "nan".into(),
            "Infinity" | "inf" => "inf".into(),
            "-Infinity" | "-inf" => "-inf".into(),
            "-0" => "-0.000000".into(),
            _ => return Err("TypeError: expected a number".into()),
        }
    } else {
        return Err("TypeError: expected a number".into());
    };
    Ok(text.into_bytes())
}
pub fn u64(value: i64) -> Result<String, String> {
    Ok((value as u64).to_string())
}
pub fn serialize_string(value: String) -> Result<Vec<u8>, String> {
    Ok(escape(value.as_bytes()))
}
pub fn serialize_bytes(value: Vec<u8>) -> Result<Vec<u8>, String> {
    Ok(escape(&value))
}
pub fn serialize_date(date: i64) -> Result<Vec<u8>, String> {
    // The Windows Python oracle's localtime rejects pre-epoch timestamps.
    if cfg!(windows) && date < 0 {
        return Err("OSError: [Errno 22] Invalid argument".into());
    }
    let t = Local
        .timestamp_opt(date, 0)
        .single()
        .ok_or("OSError: [Errno 22] Invalid argument")?;
    Ok(escape(t.format("%Y-%m-%dT%H:%M:%S").to_string().as_bytes()))
}
pub fn serialize_date_raw(date: i64) -> Result<Vec<u8>, String> {
    Ok(escape(date.to_string().as_bytes()))
}
pub fn string_allow_empty(data: String) -> Result<Vec<u8>, String> {
    Ok(if data.is_empty() {
        Vec::new()
    } else {
        escape(data.as_bytes())
    })
}
pub fn string_allow_null(data: String) -> Result<Vec<u8>, String> {
    Ok(if data.is_empty() {
        b"null".to_vec()
    } else {
        escape(data.as_bytes())
    })
}
pub fn indentation(nesting: Vec<bool>) -> Result<Vec<u8>, String> {
    Ok(vec![b' '; nesting.len()])
}
fn block(depth: usize, open: u8, close: u8, items: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let mut out = vec![open];
    let mut first = true;
    for value in items {
        if !first {
            out.push(b',');
        }
        first = false;
        out.push(b'\n');
        out.resize(out.len() + depth + 1, b' ');
        out.extend(value);
    }
    out.push(b'\n');
    out.resize(out.len() + depth, b' ');
    out.push(close);
    out
}
fn pair(key: &str, value: Vec<u8>) -> Vec<u8> {
    let mut out = escape(key.as_bytes());
    out.extend_from_slice(b": ");
    out.extend(value);
    out
}
pub fn serialize_object(
    nesting: Vec<bool>,
    keys: Vec<String>,
    values: Vec<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    Ok(block(
        nesting.len(),
        b'{',
        b'}',
        keys.into_iter()
            .zip(values)
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| pair(&k, v)),
    ))
}
pub fn serialize_array(nesting: Vec<bool>, values: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
    Ok(block(nesting.len(), b'[', b']', values))
}
pub fn serialize_text(
    nesting: Vec<bool>,
    kinds: Vec<i64>,
    texts: Vec<String>,
    extra: Vec<String>,
    objects: bool,
) -> Result<Vec<u8>, String> {
    let names = [
        "plain",
        "unknown",
        "mention",
        "hashtag",
        "bot_command",
        "link",
        "email",
        "bold",
        "italic",
        "code",
        "pre",
        "text_link",
        "mention_name",
        "phone",
        "cashtag",
        "underline",
        "strikethrough",
        "blockquote",
        "bank_card",
        "spoiler",
        "custom_emoji",
    ];
    let mut output = Vec::new();
    let mut plain = false;
    for ((kind, text), add) in kinds.into_iter().zip(texts).zip(extra) {
        let k = usize::try_from(kind)
            .ok()
            .and_then(|i| names.get(i))
            .ok_or_else(|| format!("ValueError: {kind} is not a valid TextPart.Type"))?;
        plain = kind == 0;
        if plain && !objects {
            output.push(escape(text.as_bytes()));
            continue;
        }
        let mut pairs = vec![
            pair("type", escape(k.as_bytes())),
            pair("text", escape(text.as_bytes())),
        ];
        let extra = match kind {
            10 => Some(("language", escape(add.as_bytes()))),
            11 => Some(("href", escape(add.as_bytes()))),
            12 => Some(("user_id", add.into_bytes())),
            17 => Some((
                "collapsed",
                if add.is_empty() {
                    b"false".to_vec()
                } else {
                    b"true".to_vec()
                },
            )),
            20 => Some(("document_id", escape(add.as_bytes()))),
            _ => None,
        };
        if let Some((k, v)) = extra.filter(|(_, v)| !v.is_empty()) {
            pairs.push(pair(k, v));
        }
        output.push(block(nesting.len() + 1, b'{', b'}', pairs));
    }
    if output.is_empty() {
        return Ok(if objects { b"[]".to_vec() } else { escape(b"") });
    }
    if plain && !objects && output.len() == 1 {
        return Ok(output.pop().unwrap_or_default());
    }
    Ok(block(nesting.len(), b'[', b']', output))
}
pub fn format_username(username: String) -> Result<String, String> {
    Ok(if username.is_empty() {
        username
    } else {
        format!("@{username}")
    })
}
pub fn format_file_path(path: String) -> Result<Vec<u8>, String> {
    Ok(path.into_bytes())
}

#[test]
fn desktop_escapes_controls_and_preserves_unicode_separator_continuations() {
    assert_eq!(
        serialize_string("\0\u{2028}".into()),
        Ok(b"\"\\x00\\u2028\x80\xa8\"".to_vec())
    );
}
