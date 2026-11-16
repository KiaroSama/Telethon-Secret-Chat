//! Desktop JSON byte serialization, including current valid Unicode escapes.
use super::{
    error::Result,
    format::{DateContext, DesktopNumber, iso_date_time},
    model::{File, TextPart, TextType},
};
use serde::{Deserialize, Serialize};
pub const K_OBJECT: bool = true;
pub const K_ARRAY: bool = false;
pub type Pairs = Vec<(String, Vec<u8>)>;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonContext {
    pub nesting: Vec<bool>,
    #[serde(skip)]
    pub countries: Vec<super::format::CountryInfo>,
}
pub fn number(value: impl DesktopNumber) -> Vec<u8> {
    value.desktop_number().into_bytes()
}
pub fn float_allow_null(value: f64) -> Vec<u8> {
    if value.is_finite() {
        number(value)
    } else {
        b"null".to_vec()
    }
}
pub fn u64_value(value: i64) -> u64 {
    value as u64
}
pub fn serialize_string(value: impl AsRef<[u8]>) -> Vec<u8> {
    let bytes = value.as_ref();
    let mut result = Vec::with_capacity(bytes.len() + 2);
    result.push(b'"');
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 226 && bytes.get(i + 1) == Some(&128) && matches!(bytes.get(i + 2), Some(168 | 169))
        {
            result.extend_from_slice(if bytes[i + 2] == 168 {
                b"\\u2028"
            } else {
                b"\\u2029"
            });
            i += 3;
            continue;
        }
        match b {
            b'\n' => result.extend_from_slice(b"\\n"),
            b'\r' => result.extend_from_slice(b"\\r"),
            b'\t' => result.extend_from_slice(b"\\t"),
            b'"' => result.extend_from_slice(b"\\\""),
            b'\\' => result.extend_from_slice(b"\\\\"),
            0..=31 => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                result.extend_from_slice(b"\\u00");
                result.extend([HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]);
            }
            _ => result.push(b),
        }
        i += 1;
    }
    result.push(b'"');
    result
}
pub fn serialize_date(date: i64, dates: &DateContext) -> Result<Vec<u8>> {
    Ok(serialize_string(iso_date_time(date, dates)?))
}
pub fn serialize_date_raw(date: i64) -> Vec<u8> {
    serialize_string(number(date))
}
pub fn string_allow_empty(data: &str) -> Vec<u8> {
    if data.is_empty() {
        vec![]
    } else {
        serialize_string(data)
    }
}
pub fn string_allow_null(data: &str) -> Vec<u8> {
    if data.is_empty() {
        b"null".to_vec()
    } else {
        serialize_string(data)
    }
}
pub fn indentation(context: &JsonContext) -> Vec<u8> {
    vec![b' '; context.nesting.len()]
}
pub fn serialize_object<K: AsRef<str>>(context: &JsonContext, values: &[(K, Vec<u8>)]) -> Vec<u8> {
    let depth = context.nesting.len();
    let mut result = vec![b'{'];
    let mut first = true;
    for (key, value) in values {
        if value.is_empty() {
            continue;
        }
        if !first {
            result.push(b',');
        }
        first = false;
        result.push(b'\n');
        result.extend(std::iter::repeat_n(b' ', depth + 1));
        result.extend(serialize_string(key.as_ref()));
        result.extend_from_slice(b": ");
        result.extend_from_slice(value);
    }
    result.push(b'\n');
    result.extend(std::iter::repeat_n(b' ', depth));
    result.push(b'}');
    result
}
pub fn serialize_array(context: &JsonContext, values: &[Vec<u8>]) -> Vec<u8> {
    let depth = context.nesting.len();
    let mut result = vec![b'['];
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            result.push(b',');
        }
        result.push(b'\n');
        result.extend(std::iter::repeat_n(b' ', depth + 1));
        result.extend_from_slice(value);
    }
    result.push(b'\n');
    result.extend(std::iter::repeat_n(b' ', depth));
    result.push(b']');
    result
}
pub fn text_type_name(kind: TextType) -> &'static str {
    match kind {
        TextType::Unknown => "unknown",
        TextType::Mention => "mention",
        TextType::Hashtag => "hashtag",
        TextType::BotCommand => "bot_command",
        TextType::Url => "link",
        TextType::Email => "email",
        TextType::Bold => "bold",
        TextType::Italic => "italic",
        TextType::Code => "code",
        TextType::Pre => "pre",
        TextType::Text => "plain",
        TextType::TextUrl => "text_link",
        TextType::MentionName => "mention_name",
        TextType::Phone => "phone",
        TextType::Cashtag => "cashtag",
        TextType::Underline => "underline",
        TextType::Strike => "strikethrough",
        TextType::Blockquote => "blockquote",
        TextType::BankCard => "bank_card",
        TextType::TonAddress => "ton_address",
        TextType::Spoiler => "spoiler",
        TextType::CustomEmoji => "custom_emoji",
    }
}
pub fn serialize_text(context: &mut JsonContext, data: &[TextPart], to_objects: bool) -> Vec<u8> {
    if data.is_empty() {
        return if to_objects {
            b"[]".to_vec()
        } else {
            serialize_string("")
        };
    }
    context.nesting.push(K_ARRAY);
    let mut text = Vec::new();
    for part in data {
        if part.kind == TextType::Text && !to_objects {
            text.push(serialize_string(&part.text));
            continue;
        }
        let additional = match part.kind {
            TextType::MentionName => ("user_id", part.additional.as_bytes().to_vec()),
            TextType::CustomEmoji => ("document_id", serialize_string(&part.additional)),
            TextType::Pre => ("language", serialize_string(&part.additional)),
            TextType::TextUrl => ("href", serialize_string(&part.additional)),
            TextType::Blockquote => (
                "collapsed",
                if part.additional.is_empty() {
                    b"false".to_vec()
                } else {
                    b"true".to_vec()
                },
            ),
            _ => ("none", vec![]),
        };
        text.push(serialize_object(
            context,
            &[
                ("type", serialize_string(text_type_name(part.kind))),
                ("text", serialize_string(&part.text)),
                additional,
            ],
        ));
    }
    context.nesting.pop();
    if !to_objects && data.len() == 1 && data[0].kind == TextType::Text {
        text.remove(0)
    } else {
        serialize_array(context, &text)
    }
}
pub fn format_username(username: &str) -> String {
    if username.is_empty() {
        String::new()
    } else {
        format!("@{username}")
    }
}
pub fn format_file_path(file: &File) -> Vec<u8> {
    file.relative_path.as_bytes().to_vec()
}
