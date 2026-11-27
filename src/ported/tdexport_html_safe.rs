//! Telegram Desktop export link validation (GPL-3.0).

use super::tdexport_html_qurl::{parse_strict, to_encoded};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use unicode_general_category::{GeneralCategory as G, get_general_category};

pub fn utf8_size(value: String) -> Result<i64, String> {
    Ok(value.len() as i64)
}
pub fn is_global_link(link: String) -> Result<bool, String> {
    let prefix = link.chars().take(8).collect::<String>().to_lowercase();
    Ok(prefix.starts_with("http://") || prefix.starts_with("https://"))
}
fn space(ch: char) -> bool {
    matches!(ch, '\t'..='\r' | ' ' | '\u{85}' | '\u{a0}')
        || matches!(
            get_general_category(ch),
            G::SpaceSeparator | G::LineSeparator | G::ParagraphSeparator
        )
}
pub fn qt_is_space(ch: String) -> Result<bool, String> {
    let length = ch.chars().count();
    if length != 1 {
        return Err(format!(
            "TypeError: ord() expected a character, but string of length {length} found"
        ));
    }
    Ok(ch.chars().next().is_some_and(space))
}
pub fn qt_trimmed(value: String) -> Result<String, String> {
    Ok(value.trim_matches(space).into())
}
fn encoded_control(value: &str) -> bool {
    value.as_bytes().windows(3).any(|w| {
        w[0] == b'%'
            && w[1].is_ascii_hexdigit()
            && w[2].is_ascii_hexdigit()
            && std::str::from_utf8(&w[1..])
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .is_some_and(|b| b < 32 || b == 127)
    })
}
pub fn has_encoded_control(value: String) -> Result<bool, String> {
    Ok(encoded_control(&value))
}
fn strict(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.trim_matches(space) == value
        && value.chars().all(|c| {
            !matches!(
                get_general_category(c),
                G::Control | G::LineSeparator | G::ParagraphSeparator
            )
        })
}
pub fn is_strict_target(value: String) -> Result<bool, String> {
    Ok(strict(&value))
}
fn href(value: String, message: bool) -> Result<Option<String>, String> {
    if !strict(&value) || value.starts_with("//") || encoded_control(&value) {
        return Ok(None);
    }
    let mut url = parse_strict(value.clone())?;
    if message && url["scheme"].is_null() {
        url = parse_strict(format!("https://{value}"))?;
    }
    let scheme = url["scheme"].as_str().unwrap_or_default();
    let web = matches!(scheme, "http" | "https");
    if url["valid"].as_bool() != Some(true)
        || (!web && !(message && matches!(scheme, "mailto" | "tel" | "tg")))
        || web && !url["host"].as_str().is_some_and(|s| !s.is_empty())
    {
        return Ok(None);
    }
    let result = to_encoded(url)?;
    Ok((strict(&result) && !encoded_control(&result)).then_some(result))
}
pub fn safe_message_href(value: String) -> Result<Option<String>, String> {
    href(value, true)
}
pub fn safe_http_href(value: String) -> Result<Option<String>, String> {
    href(value, false)
}
pub fn safe_email_href(address: String) -> Result<Option<String>, String> {
    if !strict(&address) || address.chars().any(|c| space(c) || "/\\:%?#&=".contains(c)) {
        return Ok(None);
    }
    let Some((local, host)) = address.split_once('@') else {
        return Ok(None);
    };
    if local.is_empty() || host.is_empty() || host.contains('@') {
        return Ok(None);
    }
    let result = format!("mailto:{address}");
    Ok(strict(&result).then_some(result))
}
pub fn safe_phone_href(phone: String) -> Result<Option<String>, String> {
    if !strict(&phone)
        || !phone.bytes().any(|b| b.is_ascii_digit())
        || !phone
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+- .()".contains(&b))
    {
        return Ok(None);
    }
    let result = format!("tel:{phone}");
    Ok(strict(&result).then_some(result))
}
fn word(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn safe_mention_href(
    mention: String,
    internal_links_domain: String,
) -> Result<Option<String>, String> {
    if !strict(&mention) || mention.len() < 2 {
        return Ok(None);
    }
    let Some(username) = mention.strip_prefix('@').filter(|s| word(s)) else {
        return Ok(None);
    };
    safe_http_href(internal_links_domain + username)
}
fn ascii_word(v: &str, min: i64, max: i64) -> bool {
    (min..=max).contains(&(v.len() as i64)) && word(v)
}
pub fn is_ascii_word(
    value: String,
    minimum_length: i64,
    maximum_length: i64,
) -> Result<bool, String> {
    Ok(ascii_word(&value, minimum_length, maximum_length))
}
fn hashtag_word(value: &str) -> bool {
    let count = value.chars().count();
    (2..=64).contains(&count)
        && value.chars().all(|c| {
            matches!(
                get_general_category(c),
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
            )
        })
        && value
            .chars()
            .any(|c| get_general_category(c) != G::DecimalNumber)
}
pub fn is_unicode_hashtag_word(value: String) -> Result<bool, String> {
    Ok(hashtag_word(&value))
}
fn hashtag(value: &str) -> bool {
    value.split_once('@').map_or_else(
        || hashtag_word(value),
        |(name, rest)| hashtag_word(name) && ascii_word(rest, 1, 32),
    )
}
fn command(value: &str) -> bool {
    value.split_once('@').map_or_else(
        || ascii_word(value, 1, 64),
        |(name, rest)| ascii_word(name, 1, 64) && ascii_word(rest, 5, 32),
    )
}
fn cash(value: &str) -> bool {
    (1..=8).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_uppercase())
}
pub fn is_hashtag_action(value: String) -> Result<bool, String> {
    Ok(hashtag(&value))
}
pub fn is_bot_command_action(value: String) -> Result<bool, String> {
    Ok(command(&value))
}
pub fn is_cashtag_action(value: String) -> Result<bool, String> {
    Ok(cash(&value))
}
fn valid_action(value: &str, prefix: &str) -> bool {
    match prefix {
        "#" => hashtag(value),
        "/" => command(value),
        "$" => cash(value),
        _ => false,
    }
}
pub fn is_valid_action_value(value: String, prefix: String) -> Result<bool, String> {
    Ok(valid_action(&value, &prefix))
}
pub fn safe_action_data(target: String, prefix: String) -> Result<Option<String>, String> {
    if !strict(&target) || target.len() < 2 || !matches!(prefix.as_str(), "#" | "/" | "$") {
        return Ok(None);
    }
    Ok(target
        .strip_prefix(&prefix)
        .filter(|v| valid_action(v, &prefix) && strict(v))
        .map(str::to_owned))
}
fn decimal(v: &str) -> bool {
    !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())
}
pub fn is_ascii_decimal(value: String) -> Result<bool, String> {
    Ok(decimal(&value))
}
pub fn safe_relative_emoji_href(
    path: String,
    relative_base: String,
) -> Result<Option<String>, String> {
    if !strict(&path)
        || path.contains(['\\', ':', '%', '?', '#'])
        || path == "(unavailable)"
        || decimal(&path)
        || path.split('/').any(|p| matches!(p, "" | "." | ".."))
    {
        return Ok(None);
    }
    let result = relative_base + &path;
    Ok(strict(&result).then_some(result))
}
pub fn anchor_token(name: String) -> Result<String, String> {
    if name.is_empty() {
        return Ok("empty".into());
    }
    let data = name.as_bytes();
    if data.len() <= 72 {
        return Ok(URL_SAFE_NO_PAD.encode(data));
    }
    let digest = Sha256::digest(data);
    Ok(format!(
        "{}-{}",
        URL_SAFE_NO_PAD.encode(&data[..48]),
        digest[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

#[test]
fn link_validation_preserves_qurl_normalization_and_refuses_encoded_controls() {
    assert_eq!(
        safe_message_href("HTTPS://Example.TEST/%7e".into()),
        Ok(Some("https://example.test/~".into()))
    );
    assert_eq!(
        safe_message_href("https://example.test/%0a".into()),
        Ok(None)
    );
}
