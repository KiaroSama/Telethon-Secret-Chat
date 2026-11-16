//! Desktop v7.2.10 export_settings: flags, defaults and validation.
use super::error::{ExportError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub const MAX_FILE_SIZE: u64 = 4000 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Html,
    Json,
    Both,
}
impl TryFrom<&str> for Format {
    type Error = ExportError;
    fn try_from(value: &str) -> Result<Self> {
        match value {
            "html" => Ok(Self::Html),
            "json" => Ok(Self::Json),
            "both" => Ok(Self::Both),
            _ => Err(ExportError::InvalidOptions),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSettings {
    pub types: u32,
    pub size_limit: u64,
}
impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            types: Self::PHOTO,
            size_limit: 8 * 1024 * 1024,
        }
    }
}
impl MediaSettings {
    pub const PHOTO: u32 = 1;
    pub const VIDEO: u32 = 2;
    pub const VOICE_MESSAGE: u32 = 4;
    pub const VIDEO_MESSAGE: u32 = 8;
    pub const STICKER: u32 = 16;
    pub const GIF: u32 = 32;
    pub const FILE: u32 = 64;
    pub const MEDIA_MASK: u32 = 15;
    pub const ALL_MASK: u32 = 127;
    pub fn validate(&self) -> bool {
        self.types & !Self::ALL_MASK == 0 && self.size_limit <= MAX_FILE_SIZE
    }
    pub fn select(
        names: impl IntoIterator<Item = impl AsRef<str>>,
        size_limit: u64,
    ) -> Result<Self> {
        let mut types = 0;
        for name in names {
            types |= match name.as_ref() {
                "photo" => Self::PHOTO,
                "video" => Self::VIDEO,
                "voice_message" => Self::VOICE_MESSAGE,
                "video_message" => Self::VIDEO_MESSAGE,
                "sticker" => Self::STICKER,
                "gif" => Self::GIF,
                "file" => Self::FILE,
                _ => return Err(ExportError::InvalidOptions),
            };
        }
        let result = Self { types, size_limit };
        if !result.validate() {
            return Err(ExportError::InvalidOptions);
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub path: PathBuf,
    pub force_sub_path: bool,
    pub format: Format,
    pub types: u32,
    pub full_chats: u32,
    pub media: MediaSettings,
    pub single_peer: Option<i64>,
    pub single_peer_input: Option<serde_json::Value>,
    pub single_peer_from: i64,
    pub single_peer_till: i64,
    pub single_topic_root_id: i64,
    pub single_topic_peer_id: i64,
    pub single_topic_title: String,
    pub available_at: i64,
    /// None uses the machine's local zone. An explicit IANA zone makes exported dates reproducible.
    pub time_zone: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            force_sub_path: false,
            format: Format::Html,
            types: 0x001 | 0x002 | 0x004 | 0x800 | 0x1000 | 0x020 | 0x080,
            full_chats: 0x020 | 0x040,
            media: MediaSettings::default(),
            single_peer: None,
            single_peer_input: None,
            single_peer_from: 0,
            single_peer_till: 0,
            single_topic_root_id: 0,
            single_topic_peer_id: 0,
            single_topic_title: String::new(),
            available_at: 0,
            time_zone: None,
        }
    }
}
impl Settings {
    pub const PERSONAL_CHATS: u32 = 0x020;
    pub const BOT_CHATS: u32 = 0x040;
    pub const PRIVATE_GROUPS: u32 = 0x080;
    pub const PUBLIC_GROUPS: u32 = 0x100;
    pub const PRIVATE_CHANNELS: u32 = 0x200;
    pub const PUBLIC_CHANNELS: u32 = 0x400;
    pub const ANY_CHATS_MASK: u32 = 0x7e0;
    pub const ALL_MASK: u32 = 0x1fff;
    pub fn only_single_peer(&self) -> bool {
        self.single_peer.is_some()
    }
    pub fn only_single_topic(&self) -> bool {
        self.only_single_peer() && self.single_topic_root_id != 0
    }
    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        if self.only_single_peer() {
            result.types = Self::ANY_CHATS_MASK;
            result.full_chats = Self::ANY_CHATS_MASK;
        }
        result
    }
    pub fn validate(&self) -> bool {
        self.types & !Self::ALL_MASK == 0
            && self.full_chats & !Self::ALL_MASK == 0
            && self.full_chats & (Self::PERSONAL_CHATS | Self::BOT_CHATS)
                == Self::PERSONAL_CHATS | Self::BOT_CHATS
            && self.full_chats & (Self::PUBLIC_GROUPS | Self::PUBLIC_CHANNELS) == 0
            && matches!(self.format, Format::Html | Format::Json)
            && self.media.validate()
            && !(self.single_peer_till > 0 && self.single_peer_till <= self.single_peer_from)
    }
    pub fn includes_date(&self, date: i64) -> bool {
        date >= self.single_peer_from
            && (self.single_peer_till == 0 || date < self.single_peer_till)
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub countries: Vec<super::format::CountryInfo>,
    pub internal_links_domain: String,
    pub about_telegram: String,
    pub about_contacts: String,
    pub about_frequent: String,
    pub about_sessions: String,
    pub about_web_sessions: String,
    pub about_chats: String,
    pub about_left_chats: String,
}
