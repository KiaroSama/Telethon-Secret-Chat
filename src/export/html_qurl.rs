//! StrictMode/FullyEncoded QUrl subset from Qt 5.15.19 (LGPL-3.0).
//! Keeps Desktop URL spelling, legacy IPv4 and IDNA2003 behavior across platforms.
use serde::{Deserialize, Serialize};
const DELIMITERS: &str = ":@][/?#\"<>^\\|{}";
fn hex(c: char) -> Option<u8> {
    c.to_digit(16)
        .filter(|_| c.is_ascii_hexdigit())
        .map(|n| n as u8)
}
fn percent(text: &[char], i: usize) -> Option<u8> {
    Some(hex(*text.get(i + 1)?)? * 16 + hex(*text.get(i + 2)?)?)
}
fn actions(encode: bool, modifications: &str) -> [u8; 96] {
    let mut table = [1; 96];
    for (code, action) in (32u8..128).zip(table.iter_mut()) {
        let c = char::from(code);
        *action = if c.is_ascii_alphanumeric() || "-._~".contains(c) {
            0
        } else if " \"%<>\\^`{|}\x7f".contains(c) {
            2
        } else {
            1
        };
    }
    if !encode {
        table[0] = 0;
    }
    for c in modifications.bytes() {
        table[usize::from(c - 32)] = if encode { 2 } else { 0 };
    }
    table
}
fn recode_run(text: &[char], encode: bool, table: &[u8; 96], retry: bool) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        let code = u32::from(c);
        let mut action = 2;
        if (32..128).contains(&code) {
            action = table[(code - 32) as usize];
            if action != 2 {
                result.push(c);
                i += 1;
                continue;
            }
        }
        if c == '%' && retry {
            result.push_str("%25");
            i += 1;
            continue;
        }
        let mut decoded = code;
        if c == '%' {
            let Some(value) = percent(text, i) else {
                return recode_run(text, encode, table, true);
            };
            decoded = u32::from(value);
            if value >= 128 {
                if !encode {
                    let need = match value {
                        0xc2..=0xdf => 1,
                        0xe0..=0xef => 2,
                        0xf0..=0xf4 => 3,
                        _ => 0,
                    };
                    if need > 0 {
                        let mut bytes = vec![value];
                        let mut valid = true;
                        for n in 1..=need {
                            let at = i + 3 * n;
                            if text.get(at) != Some(&'%') {
                                valid = false;
                                break;
                            }
                            if let Some(b) = percent(text, at) {
                                bytes.push(b);
                            } else {
                                valid = false;
                                break;
                            }
                        }
                        if valid
                            && let Ok(run) = std::str::from_utf8(&bytes)
                            && let Some(ch) = run.chars().next()
                        {
                            let code = u32::from(ch);
                            if !(0xfdd0..=0xfdef).contains(&code) && (code & 0xfffe) != 0xfffe {
                                result.push(ch);
                                i += 3 * (need + 1);
                                continue;
                            }
                        }
                    }
                }
                action = 1;
            } else if value >= 32 {
                action = table[usize::from(value - 32)];
            }
        } else if code >= 128 {
            if encode {
                for b in c.to_string().bytes() {
                    result.push_str(&format!("%{b:02X}"));
                }
            } else {
                result.push(c);
            }
            i += 1;
            continue;
        }
        if c == '%' && action != 0 {
            result.push('%');
            result.push(text[i + 1].to_ascii_uppercase());
            result.push(text[i + 2].to_ascii_uppercase());
            i += 3;
        } else if c == '%' {
            if let Some(ch) = char::from_u32(decoded) {
                result.push(ch);
            }
            i += 3;
        } else {
            result.push_str(&format!("%{code:02X}"));
            i += 1;
        }
    }
    result
}
pub fn recode(text: &str, encode: bool, modifications: &str) -> String {
    recode_run(
        &text.chars().collect::<Vec<_>>(),
        encode,
        &actions(encode, modifications),
        false,
    )
}
fn validate_component(text: &str, user_info: bool) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    for (i, c) in chars.iter().enumerate() {
        if !c.is_ascii() {
            continue;
        }
        if *c == '%' {
            if percent(&chars, i).is_none() {
                return false;
            }
        } else if *c <= ' ' || "\"<>\\^`{|}\x7f".contains(*c) || user_info && "/?#[]@".contains(*c)
        {
            return false;
        }
    }
    true
}
fn parse_ip4(text: &str) -> Option<String> {
    if !text.is_ascii() {
        return None;
    }
    let parts = text.split('.').collect::<Vec<_>>();
    if parts.len() > 4 {
        return None;
    }
    let mut address = 0u64;
    for (i, raw) in parts.iter().enumerate() {
        let part = raw
            .trim_start_matches(char::is_whitespace)
            .strip_prefix('+')
            .unwrap_or_else(|| raw.trim_start_matches(char::is_whitespace));
        if part.is_empty() {
            return None;
        }
        let (value, radix) = if part.starts_with("0x") || part.starts_with("0X") {
            (&part[2..], 16)
        } else if part.starts_with('0') {
            (part, 8)
        } else {
            (part, 10)
        };
        if value.is_empty()
            || !value.bytes().all(|b| match radix {
                16 => b.is_ascii_hexdigit(),
                8 => matches!(b, b'0'..=b'7'),
                _ => b.is_ascii_digit(),
            })
        {
            return None;
        }
        let value = u64::from_str_radix(value, radix).ok()?;
        let bits = if i == parts.len() - 1 { 32 - 8 * i } else { 8 };
        if value >> bits != 0 || value > u64::from(u32::MAX) {
            return None;
        }
        address = (address << bits) | value;
    }
    Some(std::net::Ipv4Addr::from(address as u32).to_string())
}
fn parse_ip6(text: &str) -> Option<String> {
    let mut text = text;
    let mut zone = "";
    if text.matches("%25").count() == 1 {
        (text, zone) = text.split_once("%25")?;
        if zone.is_empty() {
            return None;
        }
    }
    if text.is_empty() || text.contains('%') || !text.is_ascii() {
        return None;
    }
    let ip = text.parse::<std::net::Ipv6Addr>().ok()?;
    let packed = ip.octets();
    let formatted = if packed[..10] == [0; 10] && packed[10..12] == [255, 255] {
        format!(
            "::ffff:{}",
            std::net::Ipv4Addr::new(packed[12], packed[13], packed[14], packed[15])
        )
    } else if packed[..12] == [0; 12] && packed[12..15] != [0; 3] {
        format!(
            "::{}",
            std::net::Ipv4Addr::new(packed[12], packed[13], packed[14], packed[15])
        )
    } else {
        ip.to_string()
    };
    Some(format!(
        "[{formatted}{}]",
        if zone.is_empty() {
            String::new()
        } else {
            format!("%25{zone}")
        }
    ))
}
fn std3(label: &str) -> bool {
    label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn ace_label(label: &str) -> Option<String> {
    let lowered = label.to_ascii_lowercase();
    let simple = label.is_ascii() && !(label.len() > 6 && lowered.starts_with("xn--"));
    if simple {
        return std3(&lowered).then_some(lowered);
    }
    let prepped = stringprep::nameprep(&lowered).ok()?;
    let ace = if prepped.is_ascii() {
        prepped.into_owned()
    } else {
        format!("xn--{}", punycode::encode(&prepped).ok()?)
    };
    std3(&ace).then_some(ace)
}
fn ace(domain: &str) -> Option<String> {
    if domain.is_empty() {
        return Some(String::new());
    }
    let labels = domain
        .split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}'])
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if label.is_empty() {
            if i == labels.len() - 1 {
                result.push(String::new());
                break;
            }
            return None;
        }
        result.push(ace_label(label)?);
    }
    Some(result.join("."))
}
fn set_host(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some(String::new());
    }
    if value.starts_with('[') {
        let inner = value.strip_suffix(']')?.strip_prefix('[')?;
        if let Some(tail) = inner.strip_prefix('v') {
            if value.len() <= 5 {
                return None;
            }
            let (version, address) = tail.split_once('.')?;
            if version.len() != 1
                || !version.bytes().all(|b| b.is_ascii_hexdigit())
                || address.is_empty()
                || !address
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!$&'()*+,;=:-._~".contains(&b))
            {
                return None;
            }
            return Some(format!("[v{}.{}]", version.to_ascii_uppercase(), address));
        }
        return parse_ip6(inner);
    }
    if let Some(ip) = parse_ip4(value) {
        return Some(ip);
    }
    let ace = ace(value)?;
    if ace.is_empty() {
        None
    } else {
        Some(parse_ip4(&ace).unwrap_or(ace))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedUrl {
    pub valid: bool,
    pub scheme: Option<String>,
    pub user_name: Option<String>,
    pub password: Option<String>,
    pub host: Option<String>,
    pub port: i32,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
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
fn scheme(value: &str) -> Option<String> {
    let first = value.bytes().next()?;
    if !first.is_ascii_alphabetic()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}
fn set_authority(url: &mut ParsedUrl, auth: &str) -> bool {
    url.host = Some(String::new());
    if auth.is_empty() {
        return true;
    }
    let mut start = 0;
    if let Some(at) = auth.find('@') {
        let user_info = &auth[..at];
        let (user, password) = user_info
            .split_once(':')
            .map_or((user_info, None), |(u, p)| (u, Some(p)));
        url.user_name = Some(recode(user, false, DELIMITERS));
        url.password = password.map(|p| recode(p, false, &DELIMITERS[1..]));
        if !validate_component(user_info, true) {
            return false;
        }
        start = at + 1;
    }
    let mut colon = auth[start..].rfind(':').map(|i| start + i);
    if let Some(c) = colon
        && auth[start..].starts_with('[')
        && auth[start..].find(']').is_none_or(|i| i + start > c)
    {
        colon = None;
    }
    if let Some(c) = colon
        && c < auth.len() - 1
    {
        let mut port = 0u32;
        for b in auth[c + 1..].bytes() {
            if !b.is_ascii_digit() {
                return false;
            }
            port = port * 10 + u32::from(b - b'0');
            if port > 65535 {
                return false;
            }
        }
        if port > 65535 {
            return false;
        }
        url.port = port as i32;
    }
    let raw = &auth[start..colon.unwrap_or(auth.len())];
    let Some(host) = set_host(raw) else {
        return false;
    };
    if !validate_component(raw, false) {
        return false;
    }
    url.host = Some(host);
    true
}
pub fn parse_strict(text: &str) -> ParsedUrl {
    let mut url = ParsedUrl::default();
    let (mut colon, mut question, mut hash) = (None, None, None);
    for (i, c) in text.char_indices() {
        if c == '#' {
            hash = Some(i);
            break;
        }
        if question.is_none() {
            if c == ':' && colon.is_none() {
                colon = Some(i);
            } else if c == '?' {
                question = Some(i);
            }
        }
    }
    let mut hier_start = 0;
    if let Some(c) = colon {
        url.scheme = scheme(&text[..c]);
        if url.scheme.is_some() {
            hier_start = c + 1;
        }
    }
    let hier_end = [question, hash, Some(text.len())]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(text.len());
    let path_start = if text[hier_start..hier_end].starts_with("//") {
        let authority_end = text[hier_start + 2..hier_end]
            .find('/')
            .map_or(hier_end, |i| hier_start + 2 + i);
        if !set_authority(&mut url, &text[hier_start + 2..authority_end]) {
            url.valid = false;
            return url;
        }
        authority_end
    } else {
        hier_start
    };
    let raw_path = &text[path_start..hier_end];
    url.path = recode(raw_path, false, &DELIMITERS[5..]);
    let query = question.map(|i| &text[i + 1..hash.unwrap_or(text.len())]);
    let fragment = hash.map(|i| &text[i + 1..]);
    url.query = query.map(|q| recode(q, false, &DELIMITERS[6..]));
    url.fragment = fragment.map(|f| recode(f, false, &DELIMITERS[7..]));
    if [Some(raw_path), query, fragment]
        .into_iter()
        .flatten()
        .any(|c| !validate_component(c, false))
    {
        url.valid = false;
        return url;
    }
    url.valid = if url.path.is_empty() {
        url.scheme.is_some() || url.host.is_some() || url.query.is_some() || url.fragment.is_some()
    } else if url.path.starts_with('/') {
        url.host.is_some() || !url.path.starts_with("//")
    } else if url.host.is_some() {
        false
    } else if url.scheme.is_some() {
        true
    } else {
        url.path
            .find(':')
            .is_none_or(|c| url.path.find('/').is_some_and(|s| s < c))
    };
    url
}
pub fn to_encoded(url: &ParsedUrl) -> String {
    let mut out = String::new();
    if let Some(scheme) = &url.scheme {
        out.push_str(scheme);
        out.push(':');
    }
    if let Some(host) = &url.host {
        out.push_str("//");
        if let Some(user) = &url.user_name {
            out.push_str(&recode(user, true, ":@][/?#"));
            if let Some(password) = &url.password {
                out.push(':');
                out.push_str(&recode(password, true, "@][/?#"));
            }
            out.push('@');
        }
        out.push_str(&if host.starts_with('[') {
            recode(host, true, "")
        } else {
            host.clone()
        });
        if url.port != -1 {
            out.push(':');
            out.push_str(&url.port.to_string());
        }
    }
    out.push_str(&recode(&url.path, true, "?#"));
    if let Some(query) = &url.query {
        out.push('?');
        out.push_str(&recode(query, true, "#"));
    }
    if let Some(fragment) = &url.fragment {
        out.push('#');
        out.push_str(&recode(fragment, true, "#"));
    }
    out
}
