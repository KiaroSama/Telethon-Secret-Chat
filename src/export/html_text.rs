//! Desktop HTML escaping, entities, tag stack, names, dates and userpics.
use super::{
    error::{ExportError, Result},
    format::DateContext,
    html_safe::{qt_trimmed, safe_message_href},
    model::*,
    model_peers::{Peer, PeerUser, Peers},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub fn serialize_string(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        match c {
            '\n' | '\u{2028}' | '\u{2029}' => result.push_str("<br>"),
            '"' => result.push_str("&quot;"),
            '&' => result.push_str("&amp;"),
            '\'' => result.push_str("&apos;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '\0'..='\u{1f}' => result.push_str(&format!("&#x{:02X};", u32::from(c))),
            _ => result.push(c),
        }
    }
    result
}
pub fn serialize_list(values: &[String]) -> String {
    if values.len() < 2 {
        values.first().cloned().unwrap_or_default()
    } else {
        format!(
            "{} and {}",
            values[..values.len() - 1].join(", "),
            values.last().map(String::as_str).unwrap_or("")
        )
    }
}
/// QByteArray::mid: intentionally permits cutting a UTF-8 sequence in half.
pub fn byte_mid(value: &str, position: isize) -> Vec<u8> {
    let bytes = value.as_bytes();
    let at = if position < 0 {
        bytes.len().saturating_sub(position.unsigned_abs())
    } else {
        position as usize
    };
    bytes[at.min(bytes.len())..].to_vec()
}
pub fn format_text_link(text: &str, target: &str) -> String {
    safe_message_href(target).map_or_else(
        || text.into(),
        |href| format!("<a href=\"{}\">{text}</a>", serialize_string(&href)),
    )
}
pub fn format_custom_emoji(emoji: &str, text: &str, base: &str) -> String {
    let start = if emoji.is_empty() {
        "<a href=\"\" onclick=\"return ShowNotLoadedEmoji();\">".into()
    } else if emoji == TextPart::UNAVAILABLE_EMOJI {
        "<a href=\"\" onclick=\"return ShowNotAvailableEmoji();\">".into()
    } else {
        format!("<a href = \"{base}{emoji}\">")
    };
    format!("{start}{text}</a>")
}
pub fn format_text(data: &[TextPart], domain: &str, base: &str) -> Vec<u8> {
    let mut result = Vec::new();
    for part in data {
        let text = serialize_string(&part.text);
        use TextType as T;
        let fragment = match part.kind {
            T::Text | T::Unknown | T::BankCard => text,
            T::Mention => {
                result.extend_from_slice(format!("<a href=\"{domain}").as_bytes());
                result.extend(byte_mid(&text, 1));
                result.extend_from_slice(format!("\">{text}</a>").as_bytes());
                continue;
            }
            T::Hashtag | T::BotCommand | T::Cashtag => {
                let function = match part.kind {
                    T::Hashtag => "ShowHashtag",
                    T::BotCommand => "ShowBotCommand",
                    _ => "ShowCashtag",
                };
                let mut target = vec![b'"'];
                target.extend(byte_mid(&text, 1));
                target.push(b'"');
                result.extend_from_slice(
                    format!("<a href=\"\" onclick=\"return {function}(").as_bytes(),
                );
                result.extend(serialize_bytes(&target));
                result.extend_from_slice(format!(")\">{text}</a>").as_bytes());
                continue;
            }
            T::Url => format_text_link(&text, &part.text),
            T::Email => format!("<a href=\"mailto:{text}\">{text}</a>"),
            T::TextUrl => format_text_link(&text, &part.additional),
            T::MentionName => {
                format!("<a href=\"\" onclick=\"return ShowMentionName()\">{text}</a>")
            }
            T::Phone => format!("<a href=\"tel:{text}\">{text}</a>"),
            T::Spoiler => format!(
                "<span class=\"spoiler hidden\" onclick=\"ShowSpoiler(this)\"><span aria-hidden=\"true\">{text}</span></span>"
            ),
            T::CustomEmoji => format_custom_emoji(&part.additional, &text, base),
            _ => {
                let tag = match part.kind {
                    T::Bold => "strong",
                    T::Italic => "em",
                    T::Code => "code",
                    T::Pre => "pre",
                    T::Underline => "u",
                    T::Strike => "s",
                    T::Blockquote => "blockquote",
                    _ => "",
                };
                format!("<{tag}>{text}</{tag}>")
            }
        };
        result.extend(fragment.into_bytes());
    }
    result
}
pub fn display_date(date: i64, previous: i64, dates: &DateContext) -> Result<bool> {
    Ok(previous == 0 || dates.datetime(date)?.date() != dates.datetime(previous)?.date())
}
pub fn month_name(index: i8) -> &'static str {
    [
        "Unknown",
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(index as usize)
    .copied()
    .unwrap_or("Unknown")
}
pub fn format_date_text(date: i64, dates: &DateContext) -> Result<String> {
    let value = dates.datetime(date)?;
    Ok(format!(
        "{} {} {}",
        value.day(),
        month_name(value.month()),
        value.year()
    ))
}
pub fn format_time_text(date: i64, dates: &DateContext) -> Result<String> {
    let value = dates.datetime(date)?;
    Ok(format!("{:02}:{:02}", value.hour(), value.minute()))
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlContext {
    pub tags: Vec<(String, bool)>,
    #[serde(skip)]
    pub countries: Vec<super::format::CountryInfo>,
}
impl HtmlContext {
    pub fn push_tag(&mut self, tag: &str, attributes: &BTreeMap<String, String>) -> String {
        let block = !attributes.contains_key("inline");
        let empty = attributes.contains_key("empty");
        let mut inner = String::new();
        for (name, value) in attributes {
            if matches!(name.as_str(), "inline" | "empty") {
                continue;
            }
            inner.push_str(&format!(" {name}=\"{}\"", serialize_string(value)));
        }
        let result = format!(
            "{}<{tag}{inner}{}>{}",
            if block {
                format!("\n{}", self.indent())
            } else {
                String::new()
            },
            if empty { "/" } else { "" },
            if block { "\n" } else { "" }
        );
        if !empty {
            self.tags.push((tag.into(), block));
        }
        result
    }
    pub fn pop_tag(&mut self) -> Result<String> {
        let (tag, block) = self.tags.pop().ok_or(ExportError::WriterState)?;
        Ok(format!(
            "{}</{tag}>{}",
            if block {
                format!("\n{}", self.indent())
            } else {
                String::new()
            },
            if block { "\n" } else { "" }
        ))
    }
    pub fn indent(&self) -> String {
        " ".repeat(self.tags.len())
    }
    pub fn empty(&self) -> bool {
        self.tags.is_empty()
    }
}
pub fn attrs<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserpicData {
    pub color_index: usize,
    pub pixel_size: u32,
    pub image_link: String,
    pub large_link: String,
    pub first_name: String,
    pub last_name: String,
    pub tooltip: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryData {
    pub image_link: String,
    pub large_link: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeersMap {
    pub data: Peers,
}
impl PeersMap {
    pub fn peer(&self, id: i64) -> Peer {
        self.data.get(&id).cloned().unwrap_or_default()
    }
    pub fn user(&self, id: i64) -> PeerUser {
        self.peer(peer_from_user(id))
            .user()
            .cloned()
            .unwrap_or_default()
    }
    pub fn wrap_peer_name(&self, id: i64) -> String {
        let name = self.peer(id).name();
        if name.is_empty() {
            "Deleted".into()
        } else {
            serialize_string(&name)
        }
    }
    pub fn wrap_user_name(&self, id: i64) -> String {
        let name = self.user(id).name();
        if name.is_empty() {
            "Deleted Account".into()
        } else {
            serialize_string(&name)
        }
    }
    pub fn wrap_user_names(&self, ids: &[i64]) -> String {
        serialize_list(
            &ids.iter()
                .map(|id| self.wrap_user_name(*id))
                .collect::<Vec<_>>(),
        )
    }
}
pub fn fill_userpic_names_from_peer(data: &mut UserpicData, peer: &Peer) {
    if let Some(user) = peer.user() {
        data.first_name = user.info.first_name.clone();
        data.last_name = user.info.last_name.clone();
    } else {
        data.first_name = peer.name();
    }
}
pub fn fill_userpic_names_from_full(data: &mut UserpicData, full: &str) {
    let mut names = full.split(' ');
    data.first_name = names.next().unwrap_or("").into();
    for name in names {
        if name.is_empty() {
            continue;
        }
        if !data.last_name.is_empty() {
            data.last_name.push(' ');
        }
        data.last_name.push_str(name);
    }
}
pub fn compose_name(data: &UserpicData, empty: &str) -> String {
    if data.first_name.is_empty() && data.last_name.is_empty() {
        empty.into()
    } else {
        format!("{} {}", data.first_name, data.last_name)
    }
}
fn initial(value: &str) -> String {
    qt_trimmed(value)
        .chars()
        .next()
        .map_or_else(String::new, |c| {
            serialize_string(&if u32::from(c) > 65535 {
                "?".into()
            } else {
                c.to_string()
            })
        })
}
#[derive(Debug, Clone, Default)]
pub struct WrapBase {
    pub base: String,
    pub context: HtmlContext,
}
impl WrapBase {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            context: HtmlContext::default(),
        }
    }
    pub fn push_tag(&mut self, tag: &str, attributes: &BTreeMap<String, String>) -> String {
        self.context.push_tag(tag, attributes)
    }
    pub fn pop_tag(&mut self) -> Result<String> {
        self.context.pop_tag()
    }
    pub fn indent(&self) -> String {
        self.context.indent()
    }
    pub fn push_div(&mut self, class: &str, style: &str) -> String {
        let mut values = attrs([("class", class)]);
        if !style.is_empty() {
            values.insert("style".into(), style.into());
        }
        self.push_tag("div", &values)
    }
    pub fn relative_path(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
    pub fn push_userpic(&mut self, userpic: &UserpicData) -> Result<String> {
        let size = format!("{}px", userpic.pixel_size);
        let mut result = String::new();
        if !userpic.large_link.is_empty() {
            result += &self.push_tag(
                "a",
                &attrs([
                    ("class", "userpic_link"),
                    ("href", &self.relative_path(&userpic.large_link)),
                ]),
            );
        }
        let style = format!("width: {size}; height: {size}");
        if !userpic.image_link.is_empty() {
            result += &self.push_tag(
                "img",
                &attrs([
                    ("class", "userpic"),
                    ("style", &style),
                    ("src", &self.relative_path(&userpic.image_link)),
                    ("empty", ""),
                ]),
            );
        } else {
            result += &self.push_div(
                &format!("userpic userpic{}", userpic.color_index + 1),
                &style,
            );
            let mut attributes = attrs([
                ("class", "initials"),
                ("style", &format!("line-height: {size}")),
            ]);
            if !userpic.tooltip.is_empty() {
                attributes.insert("title".into(), userpic.tooltip.clone());
            }
            result += &self.push_tag("div", &attributes);
            result += &initial(&userpic.first_name);
            result += &initial(&userpic.last_name);
            result += &self.pop_tag()?;
            result += &self.pop_tag()?;
        }
        if !userpic.large_link.is_empty() {
            result += &self.pop_tag()?;
        }
        Ok(result)
    }
}

// Desktop escapes QByteArray controls without repairing split UTF-8 sequences.
fn serialize_bytes(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &byte in value {
        match byte {
            b'\n' => out.extend_from_slice(b"<br>"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'\'' => out.extend_from_slice(b"&apos;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            0..=31 => out.extend_from_slice(format!("&#x{byte:02X};").as_bytes()),
            _ => out.push(byte),
        }
    }
    out
}
