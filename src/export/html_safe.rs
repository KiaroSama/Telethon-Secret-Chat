//! Desktop HTML target validation and rich action values.
use super::{
    html_qurl::{parse_strict, to_encoded},
    model::TextPart,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use unicode_general_category::{GeneralCategory as G, get_general_category as category};
pub const RICH_TARGET_MAX_BYTES: usize = 4096;
pub fn utf8_size(value: &str) -> usize {
    value.len()
}
pub fn is_global_link(link: &str) -> bool {
    let lower = link
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}
pub fn qt_is_space(c: char) -> bool {
    c == ' '
        || matches!(c, '\t'..='\r')
        || matches!(c, '\u{85}' | '\u{a0}')
        || matches!(
            category(c),
            G::SpaceSeparator | G::LineSeparator | G::ParagraphSeparator
        )
}
pub fn qt_trimmed(value: &str) -> &str {
    value.trim_matches(qt_is_space)
}
pub fn has_encoded_control(value: &str) -> bool {
    value.as_bytes().windows(3).any(|w| {
        w[0] == b'%'
            && w[1].is_ascii_hexdigit()
            && w[2].is_ascii_hexdigit()
            && u8::from_str_radix(std::str::from_utf8(&w[1..]).unwrap_or(""), 16)
                .is_ok_and(|b| b < 32 || b == 127)
    })
}
pub fn is_strict_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= RICH_TARGET_MAX_BYTES
        && qt_trimmed(value) == value
        && !value.chars().any(|c| {
            c < ' '
                || c == '\x7f'
                || matches!(
                    category(c),
                    G::Control | G::LineSeparator | G::ParagraphSeparator
                )
        })
}
fn safe_href(value: &str, prepend: bool) -> Option<String> {
    if !is_strict_target(value) || value.starts_with("//") || has_encoded_control(value) {
        return None;
    }
    let mut url = parse_strict(value);
    if prepend && url.scheme.is_none() {
        url = parse_strict(&format!("https://{value}"));
    }
    let scheme = url.scheme.as_deref()?;
    let host_scheme = matches!(scheme, "http" | "https");
    if !url.valid
        || !if prepend {
            matches!(scheme, "http" | "https" | "mailto" | "tel" | "tg")
        } else {
            host_scheme
        }
        || host_scheme && url.host.as_deref().unwrap_or("").is_empty()
    {
        return None;
    }
    let result = to_encoded(&url);
    (is_strict_target(&result) && !has_encoded_control(&result)).then_some(result)
}
pub fn safe_message_href(value: &str) -> Option<String> {
    safe_href(value, true)
}
pub fn safe_http_href(value: &str) -> Option<String> {
    safe_href(value, false)
}
pub fn safe_email_href(address: &str) -> Option<String> {
    if !is_strict_target(address) {
        return None;
    }
    let at = address.find('@')?;
    if at == 0
        || at != address.rfind('@')?
        || at + 1 == address.len()
        || address.chars().any(qt_is_space)
        || address.contains(['/', '\\', ':', '%', '?', '#', '&', '='])
    {
        return None;
    }
    let result = format!("mailto:{address}");
    is_strict_target(&result).then_some(result)
}
pub fn safe_phone_href(phone: &str) -> Option<String> {
    if !is_strict_target(phone)
        || !phone.bytes().any(|b| b.is_ascii_digit())
        || !phone
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+- .()".contains(&b))
    {
        return None;
    }
    let result = format!("tel:{phone}");
    is_strict_target(&result).then_some(result)
}
fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}
pub fn safe_mention_href(mention: &str, domain: &str) -> Option<String> {
    if !is_strict_target(mention)
        || mention.len() < 2
        || !mention.starts_with('@')
        || !mention[1..].bytes().all(word)
    {
        return None;
    }
    safe_http_href(&format!("{domain}{}", &mention[1..]))
}
pub fn is_ascii_word(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(word)
}
pub fn is_unicode_hashtag_word(value: &str) -> bool {
    if !(2..=64).contains(&value.chars().count()) {
        return false;
    }
    let mut all_decimal = true;
    for c in value.chars() {
        let cat = category(c);
        if !matches!(
            cat,
            G::UppercaseLetter
                | G::LowercaseLetter
                | G::TitlecaseLetter
                | G::ModifierLetter
                | G::OtherLetter
                | G::DecimalNumber
                | G::LetterNumber
                | G::OtherNumber
                | G::ConnectorPunctuation
                | G::NonspacingMark
                | G::SpacingMark
                | G::EnclosingMark
        ) {
            return false;
        }
        all_decimal &= cat == G::DecimalNumber;
    }
    !all_decimal
}
pub fn is_hashtag_action(value: &str) -> bool {
    let (base, rest) = value
        .split_once('@')
        .map_or((value, None), |(b, r)| (b, Some(r)));
    is_unicode_hashtag_word(base)
        && rest.is_none_or(|r| {
            value.bytes().filter(|b| *b == b'@').count() == 1 && is_ascii_word(r, 1, 32)
        })
}
pub fn is_bot_command_action(value: &str) -> bool {
    let (base, rest) = value
        .split_once('@')
        .map_or((value, None), |(b, r)| (b, Some(r)));
    is_ascii_word(base, 1, 64)
        && rest.is_none_or(|r| {
            value.bytes().filter(|b| *b == b'@').count() == 1 && is_ascii_word(r, 5, 32)
        })
}
pub fn is_cashtag_action(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8 && value.bytes().all(|b| b.is_ascii_uppercase())
}
pub fn is_valid_action_value(value: &str, prefix: &str) -> bool {
    match prefix {
        "#" => is_hashtag_action(value),
        "/" => is_bot_command_action(value),
        "$" => is_cashtag_action(value),
        _ => false,
    }
}
pub fn safe_action_data(target: &str, prefix: &str) -> Option<String> {
    if !is_strict_target(target)
        || !target.starts_with(prefix)
        || target.len() < 2
        || prefix.len() != 1
    {
        return None;
    }
    let result = &target[1..];
    (is_valid_action_value(result, prefix) && is_strict_target(result)).then(|| result.to_owned())
}
pub fn is_ascii_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}
pub fn safe_relative_emoji_href(path: &str, base: &str) -> Option<String> {
    if !is_strict_target(path)
        || path.starts_with('/')
        || path.contains(['\\', ':', '%', '?', '#'])
        || path == TextPart::UNAVAILABLE_EMOJI
        || is_ascii_decimal(path)
        || path.split('/').any(|c| matches!(c, "" | "." | ".."))
    {
        return None;
    }
    let result = format!("{base}{path}");
    is_strict_target(&result).then_some(result)
}
pub fn anchor_token(name: &str) -> String {
    if name.is_empty() {
        return "empty".into();
    }
    let data = name.as_bytes();
    if data.len() <= 72 {
        return URL_SAFE_NO_PAD.encode(data);
    }
    let digest = hex::encode(Sha256::digest(data));
    format!("{}-{}", URL_SAFE_NO_PAD.encode(&data[..48]), &digest[..16])
}
