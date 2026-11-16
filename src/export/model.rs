//! Portable export model and legacy saved/TL-JSON parsing seams.
use super::{
    error::{ExportError, Result},
    format::{DateContext, format_date_time},
    settings::MediaSettings,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const CHAT_TYPE_MASK: i64 = 0xffffffffffff;
pub const MIGRATED_MESSAGES_ID_SHIFT: i64 = -1_000_000_000;
pub fn peer_from_user(id: i64) -> i64 {
    id
}
pub fn peer_from_chat(id: i64) -> i64 {
    id | (1 << 48)
}
pub fn peer_from_channel(id: i64) -> i64 {
    id | (2 << 48)
}
pub fn peer_is_user(id: i64) -> bool {
    (id >> 48) & 255 == 0
}
pub fn peer_is_chat(id: i64) -> bool {
    (id >> 48) & 255 == 1
}
pub fn peer_is_channel(id: i64) -> bool {
    (id >> 48) & 255 == 2
}
pub fn peer_to_bare_id(id: i64) -> i64 {
    id & CHAT_TYPE_MASK
}
pub fn peer_to_user(id: i64) -> i64 {
    if peer_is_user(id) {
        peer_to_bare_id(id)
    } else {
        0
    }
}
pub fn peer_to_chat(id: i64) -> i64 {
    if peer_is_chat(id) {
        peer_to_bare_id(id)
    } else {
        0
    }
}
pub fn peer_to_channel(id: i64) -> i64 {
    if peer_is_channel(id) {
        peer_to_bare_id(id)
    } else {
        0
    }
}
pub fn parse_peer_id(value: &Value) -> i64 {
    match kind(value) {
        "peerUser" | "PeerUser" => peer_from_user(integer(value, "user_id")),
        "peerChat" | "PeerChat" => peer_from_chat(integer(value, "chat_id")),
        "peerChannel" | "PeerChannel" => peer_from_channel(integer(value, "channel_id")),
        _ => 0,
    }
}
pub fn peer_color_index(id: i64) -> usize {
    [0, 7, 4, 1, 6, 3, 5][id.rem_euclid(7) as usize]
}
pub fn peer_id_color_index(id: i64) -> usize {
    peer_color_index(peer_to_bare_id(id))
}
pub fn string_bare_peer_id(text: &str) -> u8 {
    text.bytes()
        .fold(255u8, |acc, b| acc.wrapping_mul(239).wrapping_add(b))
}
pub fn domain_application_id(text: &str) -> i64 {
    0x1000 + i64::from(string_bare_peer_id(text))
}
pub fn application_color_index(id: i64) -> usize {
    match id {
        1 | 7 => 0,
        6 | 21724 => 1,
        2834 => 2,
        2496 => 3,
        2040 => 4,
        1429 => 5,
        _ => peer_color_index(id),
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditsType {
    #[default]
    Stars,
    Ton,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditsAmount {
    whole: i64,
    nano: i32,
    kind: CreditsType,
}
impl CreditsAmount {
    pub fn make(whole: i64, nano: i64, kind: CreditsType) -> Self {
        let shifts = nano.div_euclid(1_000_000_000);
        Self {
            whole: whole.saturating_add(shifts),
            nano: nano.rem_euclid(1_000_000_000) as i32,
            kind,
        }
    }
    pub fn whole(&self) -> i64 {
        self.whole
    }
    pub fn nano(&self) -> i32 {
        self.nano
    }
    pub fn value(&self) -> f64 {
        self.whole as f64 + f64::from(self.nano) / 1e9
    }
    pub fn ton(&self) -> bool {
        self.kind == CreditsType::Ton
    }
    pub fn stars(&self) -> bool {
        !self.ton()
    }
    pub fn kind(&self) -> CreditsType {
        self.kind
    }
    pub fn empty(&self) -> bool {
        self.whole == 0 && self.nano == 0
    }
}
impl std::ops::Sub for CreditsAmount {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::make(
            self.whole.saturating_sub(other.whole),
            i64::from(self.nano) - i64::from(other.nano),
            self.kind,
        )
    }
}
pub fn credits_amount_from_tl(v: &Value) -> CreditsAmount {
    match kind(v) {
        "starsAmount" | "StarsAmount" => CreditsAmount::make(
            integer(v, "amount"),
            integer(v, "nanos"),
            CreditsType::Stars,
        ),
        "starsTonAmount" | "StarsTonAmount" => {
            CreditsAmount::make(0, integer(v, "amount"), CreditsType::Ton)
        }
        _ => CreditsAmount::default(),
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Birthday(i32);
impl Birthday {
    pub const YEAR_MIN: i32 = 1875;
    pub const YEAR_MAX: i32 = 2100;
    pub fn make(day: i32, month: i32, year: i32) -> Self {
        let valid = (year == 0 || (Self::YEAR_MIN..=Self::YEAR_MAX).contains(&year))
            && day > 0
            && match month {
                2 => {
                    day <= 28
                        || (day == 29
                            && (year == 0
                                || (year % 4 == 0 && (year % 100 != 0 || year % 400 == 0))))
                }
                4 | 6 | 9 | 11 => day <= 30,
                1 | 3 | 5 | 7 | 8 | 10 | 12 => day <= 31,
                _ => false,
            };
        Self(if valid {
            day + month * 100 + year * 10000
        } else {
            0
        })
    }
    pub fn serialize(&self) -> i32 {
        self.0
    }
    pub fn valid(&self) -> bool {
        self.0 != 0
    }
    pub fn day(&self) -> i32 {
        self.0 % 100
    }
    pub fn month(&self) -> i32 {
        self.0 / 100 % 100
    }
    pub fn year(&self) -> i32 {
        self.0 / 10000
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextType {
    #[default]
    Text,
    Unknown,
    Mention,
    Hashtag,
    BotCommand,
    Url,
    Email,
    Bold,
    Italic,
    Code,
    Pre,
    TextUrl,
    MentionName,
    Phone,
    Cashtag,
    Underline,
    Strike,
    Blockquote,
    BankCard,
    TonAddress,
    Spoiler,
    CustomEmoji,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPart {
    pub kind: TextType,
    pub text: String,
    pub additional: String,
}
impl TextPart {
    pub const UNAVAILABLE_EMOJI: &'static str = "(unavailable)";
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            kind: TextType::Text,
            text: text.into(),
            additional: String::new(),
        }
    }
}
pub fn parse_text(text: &str, entities: &[Value]) -> Vec<TextPart> {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let size = text.len();
    let mut offset = 0usize;
    let mut result = Vec::new();
    // Desktop entity offsets are QString UTF-16 positions; its substring operation
    // replaces an isolated surrogate. Keep that documented display-format rule.
    let mid = |start: usize, length: usize| {
        String::from_utf16_lossy(
            &units[start.min(units.len())..start.saturating_add(length).min(units.len())],
        )
    };
    for entity in entities {
        let start = integer(entity, "offset");
        let length = integer(entity, "length");
        if start < offset as i64
            || length <= 0
            || start
                .checked_add(length)
                .is_none_or(|end| end > size as i64)
        {
            continue;
        }
        let start = start as usize;
        let length = length as usize;
        if start > offset {
            result.push(TextPart::plain(mid(offset, start - offset)));
        }
        let name = kind(entity)
            .trim_start_matches("MessageEntity")
            .trim_start_matches("messageEntity");
        let t = match name {
            "Unknown" => TextType::Unknown,
            "Mention" => TextType::Mention,
            "Hashtag" => TextType::Hashtag,
            "BotCommand" => TextType::BotCommand,
            "Url" => TextType::Url,
            "Email" => TextType::Email,
            "Bold" => TextType::Bold,
            "Italic" => TextType::Italic,
            "Code" => TextType::Code,
            "Pre" => TextType::Pre,
            "TextUrl" => TextType::TextUrl,
            "MentionName" => TextType::MentionName,
            "Phone" => TextType::Phone,
            "Cashtag" => TextType::Cashtag,
            "Underline" => TextType::Underline,
            "Strike" => TextType::Strike,
            "Blockquote" => TextType::Blockquote,
            "BankCard" => TextType::BankCard,
            "TonAddress" => TextType::TonAddress,
            "Spoiler" => TextType::Spoiler,
            "CustomEmoji" => TextType::CustomEmoji,
            _ => TextType::Unknown,
        };
        let additional = match t {
            TextType::Pre => string(entity, "language").into(),
            TextType::TextUrl => string(entity, "url").into(),
            TextType::MentionName => integer(entity, "user_id").to_string(),
            TextType::CustomEmoji => integer(entity, "document_id").to_string(),
            TextType::Blockquote => {
                if boolean(entity, "collapsed") {
                    "1".into()
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };
        result.push(TextPart {
            kind: t,
            text: mid(start, length),
            additional,
        });
        offset = start + length;
    }
    if size > offset {
        result.push(TextPart::plain(mid(offset, size - offset)));
    }
    result
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    pub phone_number: String,
    pub is_bot: bool,
    pub is_self: bool,
    pub access_hash: Option<i64>,
}
impl User {
    pub fn name(&self) -> String {
        if self.first_name.is_empty() {
            self.last_name.clone()
        } else if self.last_name.is_empty() {
            self.first_name.clone()
        } else {
            format!("{} {}", self.first_name, self.last_name)
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.id <= 0 {
            Err(ExportError::InvalidOptions)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    #[default]
    None,
    Unavailable,
    FileType,
    FileSize,
    DateLimits,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileLocation {
    pub dc_id: i32,
    pub data: Value,
}
impl FileLocation {
    pub fn available(&self) -> bool {
        self.dc_id != 0
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub location: FileLocation,
    pub size: u64,
    pub content: Vec<u8>,
    pub suggested_path: String,
    pub relative_path: String,
    pub skip_reason: SkipReason,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub file: File,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: i64,
    pub date: i64,
    pub spoilered: bool,
    pub image: Image,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: i64,
    pub date: i64,
    pub file: File,
    pub thumb: Image,
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub sticker_emoji: String,
    pub song_performer: String,
    pub song_title: String,
    pub duration: u32,
    pub is_sticker: bool,
    pub is_animated: bool,
    pub is_video_message: bool,
    pub is_voice_message: bool,
    pub is_video_file: bool,
    pub is_audio_file: bool,
    pub spoilered: bool,
}
impl Document {
    pub fn media_type(&self) -> u32 {
        if self.is_sticker {
            MediaSettings::STICKER
        } else if self.is_video_message {
            MediaSettings::VIDEO_MESSAGE
        } else if self.is_voice_message {
            MediaSettings::VOICE_MESSAGE
        } else if self.is_animated {
            MediaSettings::GIF
        } else if self.is_video_file {
            MediaSettings::VIDEO
        } else {
            MediaSettings::FILE
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParseMediaContext {
    pub self_peer_id: i64,
    pub photos: u64,
    pub audios: u64,
    pub videos: u64,
    pub files: u64,
    pub contacts: u64,
    pub bot_id: i64,
}
pub fn prepare_file_name_date_part(date: i64, context: &DateContext) -> Result<String> {
    if date == 0 {
        Ok(String::new())
    } else {
        Ok("@".to_owned() + &format_date_time(date, false, "-", "-", "_", context)?)
    }
}
pub fn prepare_photo_file_name(index: u64, date: i64, context: &DateContext) -> Result<String> {
    Ok(format!(
        "photo_{index}{}.jpg",
        prepare_file_name_date_part(date, context)?
    ))
}
pub fn prepare_story_file_name(
    index: u64,
    date: i64,
    extension: &str,
    context: &DateContext,
) -> Result<String> {
    Ok(format!(
        "story_{index}{}{extension}",
        prepare_file_name_date_part(date, context)?
    ))
}
pub fn kind(value: &Value) -> &str {
    value.get("_").and_then(Value::as_str).unwrap_or("")
}
pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
pub fn integer(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}
pub fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}
pub fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
