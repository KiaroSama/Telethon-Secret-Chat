//! Strict QUrl parsing and FullyEncoded output derived from the frozen Python port of Qt
//! 5.15.19 (LGPL-3.0). The recording adapter projects the same nine fields into ordered JSON.

use serde_json::{Value, json};
use std::net::{Ipv4Addr, Ipv6Addr};
use unicode_normalization::UnicodeNormalization;

const ISOLATED: &str = ":@][/?#\"<>^\\|{}";
const IN_URL: &str = ":@][/?#";
const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Decode,
    Leave,
    Encode,
}

fn action(byte: u8, encode: bool, modifications: &str) -> Action {
    if byte < b' ' {
        return Action::Encode;
    }
    if modifications.as_bytes().contains(&byte) {
        return if encode {
            Action::Encode
        } else {
            Action::Decode
        };
    }
    if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) || (!encode && byte == b' ') {
        Action::Decode
    } else if b" \"%<>\\^`{|}\x7f".contains(&byte) {
        Action::Encode
    } else {
        Action::Leave
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_value(bytes: &[u8], index: usize) -> Option<u8> {
    let high = hex_value(*bytes.get(index + 1)?)?;
    let low = hex_value(*bytes.get(index + 2)?)?;
    Some((high << 4) | low)
}

fn push_percent(out: &mut String, value: u8) {
    out.push('%');
    out.push(char::from(HEX[usize::from(value >> 4)]));
    out.push(char::from(HEX[usize::from(value & 15)]));
}

fn percent_utf8(bytes: &[u8], index: usize, first: u8) -> Option<(char, usize)> {
    let len = match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let mut data = [first, 0, 0, 0];
    for (idx, byte) in data.iter_mut().enumerate().take(len).skip(1) {
        let pos = index + idx * 3;
        if bytes.get(pos) != Some(&b'%') {
            return None;
        }
        *byte = percent_value(bytes, pos)?;
    }
    let c = std::str::from_utf8(&data[..len]).ok()?.chars().next()?;
    let code = u32::from(c);
    (!(0xfdd0..=0xfdef).contains(&code) && code & 0xfffe != 0xfffe).then_some((c, len * 3))
}

fn recode(text: &str, encode: bool, modifications: &str) -> String {
    let bytes = text.as_bytes();
    let retry = bytes.iter().enumerate().any(|(i, &b)| {
        b == b'%'
            && action(b, encode, modifications) == Action::Encode
            && percent_value(bytes, i).is_none()
    });
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while pos < bytes.len() {
        let b = bytes[pos];
        if b >= 128 {
            if encode {
                push_percent(&mut out, b);
                pos += 1;
            } else if let Some(c) = text[pos..].chars().next() {
                out.push(c);
                pos += c.len_utf8();
            }
            continue;
        }
        let current = action(b, encode, modifications);
        if current != Action::Encode {
            out.push(char::from(b));
        } else if b == b'%' && !retry {
            if let Some(decoded) = percent_value(bytes, pos) {
                if decoded >= 128
                    && !encode
                    && let Some((c, len)) = percent_utf8(bytes, pos, decoded)
                {
                    out.push(c);
                    pos += len;
                    continue;
                }
                if decoded < 128 && action(decoded, encode, modifications) == Action::Decode {
                    out.push(char::from(decoded));
                } else {
                    push_percent(&mut out, decoded);
                }
                pos += 3;
                continue;
            }
        } else {
            push_percent(&mut out, b);
        }
        pos += 1;
    }
    out
}

fn valid_component(text: &str, user_info: bool) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().all(|(i, &b)| match b {
        b'%' => percent_value(bytes, i).is_some(),
        0..=32 | 0x7f => false,
        _ => !b"\"<>\\^`{|}".contains(&b) && (!user_info || !b"/?#[]@".contains(&b)),
    })
}

fn parse_ip4(text: &str) -> Option<Ipv4Addr> {
    if text.bytes().any(|b| b >= 0x7f) {
        return None;
    }
    let count = text.split('.').count();
    if count > 4 {
        return None;
    }
    let mut address = 0u64;
    for (index, part) in text.split('.').enumerate() {
        let part = part.trim_start_matches(|c: char| c.is_whitespace());
        let literal = part.strip_prefix('+').unwrap_or(part);
        let (digits, radix) = if let Some(h) = literal
            .strip_prefix("0x")
            .or_else(|| literal.strip_prefix("0X"))
        {
            (h, 16)
        } else {
            (literal, if literal.starts_with('0') { 8 } else { 10 })
        };
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        let value = u64::from_str_radix(digits, radix).ok()?;
        let bits = if index + 1 == count {
            32 - 8 * index
        } else {
            8
        };
        if value >> bits != 0 {
            return None;
        }
        address = (address << bits) | value;
    }
    Some(Ipv4Addr::from(u32::try_from(address).ok()?))
}

fn format_ip6(ip: Ipv6Addr) -> String {
    let octets = ip.octets();
    if octets[..10] == [0; 10] {
        let tail = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
        if octets[10..12] == [255, 255] {
            return format!("::ffff:{tail}");
        }
        if octets[10..12] == [0, 0] && octets[12..15] != [0; 3] {
            return format!("::{tail}");
        }
    }
    ip.to_string()
}

fn parse_ip6(text: &str) -> Option<String> {
    let (ip, zone) = if text.matches("%25").count() == 1 {
        let (ip, zone) = text.split_once("%25")?;
        if zone.is_empty() {
            return None;
        }
        (ip, Some(zone))
    } else {
        (text, None)
    };
    if ip.bytes().any(|b| b == b'%' || b >= 127) {
        return None;
    }
    let ip = format_ip6(ip.parse().ok()?);
    Some(zone.map_or_else(|| format!("[{ip}]"), |zone| format!("[{ip}%25{zone}]")))
}

fn std3(label: &str) -> bool {
    label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn nameprep(label: &str) -> Option<String> {
    use stringprep::tables as t;
    let folded = label
        .chars()
        .filter(|&c| !t::commonly_mapped_to_nothing(c))
        .flat_map(t::case_fold_for_nfkc)
        .map(|c| match c {
            // Unicode normalization corrections issued after Unicode 3.2.
            '\u{2f868}' => '\u{2136a}',
            '\u{2f874}' => '\u{5f33}',
            '\u{2f91f}' => '\u{43ab}',
            '\u{2f95f}' => '\u{7aae}',
            '\u{2f9bf}' => '\u{4d57}',
            _ => c,
        })
        .collect::<String>();
    // The 3.2 DB leaves unassigned code points untouched during normalization.
    if folded.chars().any(t::unassigned_code_point) {
        return None;
    }
    let normalized: String = folded.nfkc().collect();
    if normalized.chars().any(|c| {
        t::unassigned_code_point(c)
            || t::non_ascii_space_character(c)
            || t::non_ascii_control_character(c)
            || t::private_use(c)
            || t::non_character_code_point(c)
            || t::inappropriate_for_plain_text(c)
            || t::inappropriate_for_canonical_representation(c)
            || t::change_display_properties_or_deprecated(c)
            || t::tagging_character(c)
    }) {
        return None;
    }
    if normalized.chars().any(t::bidi_r_or_al)
        && (normalized.chars().any(t::bidi_l)
            || !normalized.chars().next().is_some_and(t::bidi_r_or_al)
            || !normalized.chars().next_back().is_some_and(t::bidi_r_or_al))
    {
        return None;
    }
    Some(normalized)
}

fn ace_label(label: &str) -> Option<String> {
    let lowered = label.to_ascii_lowercase();
    let ace = if label.is_ascii() && !(label.len() > 6 && lowered.starts_with("xn--")) {
        lowered
    } else {
        let prepped = nameprep(&lowered)?;
        if prepped.is_ascii() {
            prepped
        } else {
            format!("xn--{}", punycode::encode(&prepped).ok()?)
        }
    };
    std3(&ace).then_some(ace)
}

#[test]
fn a_single_dot_is_not_a_host() {
    assert_eq!(ace("."), None);
}

fn ace(domain: &str) -> Option<String> {
    if domain.is_empty() {
        return Some(domain.to_owned());
    }
    let mut labels = domain
        .split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}'])
        .peekable();
    let mut out = Vec::new();
    while let Some(label) = labels.next() {
        if label.is_empty() && labels.peek().is_some() {
            return None;
        }
        out.push(ace_label(label)?);
    }
    Some(out.join("."))
}

fn set_host(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some(String::new());
    }
    if value.starts_with('[') {
        let inner = value.strip_prefix('[')?.strip_suffix(']')?;
        if let Some(inner) = inner.strip_prefix('v') {
            let (version, address) = inner.split_once('.')?;
            if version.len() != 1
                || !version.bytes().all(|c| c.is_ascii_hexdigit())
                || address.is_empty()
                || !address
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!$&'()*+,;=:-._~".contains(&b))
            {
                return None;
            }
            return Some(format!("[v{}.{address}]", version.to_ascii_uppercase()));
        }
        return parse_ip6(inner);
    }
    if let Some(ip) = parse_ip4(value) {
        return Some(ip.to_string());
    }
    let encoded = ace(value)?;
    if encoded.is_empty() {
        return None;
    }
    Some(parse_ip4(&encoded).map_or(encoded, |ip| ip.to_string()))
}

struct ParsedUrl {
    valid: bool,
    scheme: Option<String>,
    user_name: Option<String>,
    password: Option<String>,
    host: Option<String>,
    port: i64,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Default for ParsedUrl {
    fn default() -> Self {
        Self {
            valid: true,
            scheme: None,
            user_name: None,
            password: None,
            host: None,
            port: -1,
            path: String::new(),
            query: None,
            fragment: None,
        }
    }
}

impl ParsedUrl {
    fn authority(&mut self, auth: &str) -> bool {
        self.host = Some(String::new());
        if auth.is_empty() {
            return true;
        }
        let host_port = if let Some((user_info, rest)) = auth.split_once('@') {
            let (name, password) = user_info
                .split_once(':')
                .map_or((user_info, None), |(n, p)| (n, Some(p)));
            self.user_name = Some(recode(name, false, ISOLATED));
            self.password = password.map(|p| recode(p, false, &ISOLATED[1..]));
            if !valid_component(user_info, true) {
                return false;
            }
            rest
        } else {
            auth
        };
        let mut colon = host_port.rfind(':');
        if let Some(i) = colon
            && host_port.starts_with('[')
            && host_port.find(']').is_none_or(|close| close > i)
        {
            colon = None;
        }
        let raw_host = if let Some(i) = colon {
            let port = &host_port[i + 1..];
            if !port.is_empty() {
                if !port.bytes().all(|b| b.is_ascii_digit()) {
                    return false;
                }
                let port = port.bytes().fold(0u32, |acc, n| {
                    acc.wrapping_mul(10).wrapping_add(u32::from(n - b'0'))
                });
                if port > u32::from(u16::MAX) {
                    return false;
                }
                self.port = i64::from(port);
            }
            &host_port[..i]
        } else {
            host_port
        };
        if let Some(host) = set_host(raw_host).filter(|_| valid_component(raw_host, false)) {
            self.host = Some(host);
            true
        } else {
            false
        }
    }

    fn validity(&self) -> bool {
        if self.path.is_empty() {
            self.scheme.is_some()
                || self.host.is_some()
                || self.query.is_some()
                || self.fragment.is_some()
        } else if self.path.starts_with('/') {
            self.host.is_some() || !self.path.starts_with("//")
        } else if self.host.is_some() {
            false
        } else {
            self.scheme.is_some()
                || self
                    .path
                    .find(':')
                    .is_none_or(|colon| self.path.find('/').is_some_and(|slash| slash < colon))
        }
    }

    fn into_value(self) -> Value {
        json!({ "valid": self.valid, "scheme": self.scheme, "user_name": self.user_name,
            "password": self.password, "host": self.host, "port": self.port, "path": self.path,
            "query": self.query, "fragment": self.fragment })
    }
}

fn scheme(text: &str) -> Option<String> {
    let mut bytes = text.bytes();
    if !bytes.next()?.is_ascii_alphabetic()
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    {
        return None;
    }
    Some(text.to_ascii_lowercase())
}

/// Preserve partial fields even when StrictMode rejects the URL.
pub fn parse_strict(text: String) -> Result<Value, String> {
    let mut url = ParsedUrl::default();
    let (hier_query, fragment) = text
        .split_once('#')
        .map_or((text.as_str(), None), |(h, f)| (h, Some(f)));
    let (hier, query) = hier_query
        .split_once('?')
        .map_or((hier_query, None), |(h, q)| (h, Some(q)));
    let mut path = hier;
    if let Some((prefix, rest)) = hier.split_once(':') {
        url.scheme = scheme(prefix);
        if url.scheme.is_some() {
            path = rest;
        }
    }
    if let Some(rest) = path.strip_prefix("//") {
        let end = rest.find('/').unwrap_or(rest.len());
        if !url.authority(&rest[..end]) {
            url.valid = false;
            return Ok(url.into_value());
        }
        path = &rest[end..];
    }
    url.path = recode(path, false, &ISOLATED[5..]);
    url.query = query.map(|q| recode(q, false, &ISOLATED[6..]));
    url.fragment = fragment.map(|f| recode(f, false, &ISOLATED[7..]));
    url.valid = [Some(path), query, fragment]
        .into_iter()
        .flatten()
        .all(|part| valid_component(part, false))
        && url.validity();
    Ok(url.into_value())
}

fn py_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::Object(o) => match o.keys().next().filter(|_| o.len() == 1).map(String::as_str) {
            Some("$tuple") => "tuple",
            Some("$set") => "set",
            Some("$bytes") => "bytes",
            Some("$int") => "int",
            Some("$num" | "$float") => "float",
            _ => "dict",
        },
    }
}

fn url_field<'a>(
    url: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    match url.get(name) {
        Some(v) if !v.is_null() => v.as_str().map(Some).ok_or_else(|| {
            format!(
                "TypeError: can only concatenate str (not \"{}\") to str",
                py_type(v)
            )
        }),
        _ => Ok(None),
    }
}

/// Join URL components, preserving absent versus present-but-empty delimiters.
pub fn to_encoded(url: Value) -> Result<String, String> {
    let ty = py_type(&url);
    let map = url.as_object().filter(|_| ty == "dict").ok_or_else(|| format!(
        "TypeError: telethon_secret_chat.tdexport.html_qurl.ParsedUrl() argument after ** must be a mapping, not {ty}"
    ))?;
    if let Some(k) = map.keys().find(|k| {
        ![
            "valid",
            "scheme",
            "user_name",
            "password",
            "host",
            "port",
            "path",
            "query",
            "fragment",
        ]
        .contains(&k.as_str())
    }) {
        return Err(format!(
            "TypeError: ParsedUrl.__init__() got an unexpected keyword argument '{k}'"
        ));
    }
    let mut out = String::new();
    if let Some(scheme) = map.get("scheme").filter(|v| !v.is_null()) {
        let scheme = scheme.as_str().ok_or_else(|| {
            format!(
                "TypeError: unsupported operand type(s) for +: '{}' and 'str'",
                py_type(scheme)
            )
        })?;
        out.push_str(scheme);
        out.push(':');
    }
    if let Some(host) = map.get("host").filter(|v| !v.is_null()) {
        out.push_str("//");
        if let Some(name) = url_field(map, "user_name")? {
            out.push_str(&recode(name, true, IN_URL));
            if let Some(pw) = url_field(map, "password")? {
                out.push(':');
                out.push_str(&recode(pw, true, &IN_URL[1..]));
            }
            out.push('@');
        }
        let host = host.as_str().ok_or_else(|| {
            format!(
                "AttributeError: '{}' object has no attribute 'startswith'",
                py_type(host)
            )
        })?;
        if host.starts_with('[') {
            out.push_str(&recode(host, true, ""));
        } else {
            out.push_str(host);
        }
        if let Some(port) = map
            .get("port")
            .filter(|v| v.as_i64() != Some(-1) && v.as_f64() != Some(-1.0))
        {
            out.push(':');
            match port {
                Value::String(s) => out.push_str(s),
                Value::Bool(b) => out.push_str(if *b { "True" } else { "False" }),
                Value::Null => out.push_str("None"),
                Value::Object(o) if o.len() == 1 && o.contains_key("$int") => {
                    out.push_str(o["$int"].as_str().unwrap_or(""))
                }
                other => out.push_str(&other.to_string()),
            }
        }
    }
    let path = match map.get("path") {
        Some(v) => v
            .as_str()
            .ok_or_else(|| format!("TypeError: object of type '{}' has no len()", py_type(v)))?,
        None => "",
    };
    out.push_str(&recode(path, true, &IN_URL[5..]));
    for (field, delimiter) in [("query", '?'), ("fragment", '#')] {
        if let Some(text) = url_field(map, field)? {
            out.push(delimiter);
            out.push_str(&recode(text, true, &IN_URL[6..]));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strict_parser_keeps_partial_fields_on_invalid_authority() {
        let url = parse_strict("https://a.test:65536/x".into());
        assert_eq!(url.map(|v| v["valid"].clone()), Ok(json!(false)));
    }

    #[test]
    fn encoder_keeps_empty_query_and_fragment_and_encodes_utf8() {
        let out = to_encoded(json!({"path": "/😄", "query": "", "fragment": ""}));
        assert_eq!(out, Ok("/%F0%9F%98%84?#".into()));
    }
}
