use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

pub const MEDIA_KINDS: [&str; 8] = [
    "photo",
    "video",
    "document",
    "audio",
    "animation",
    "sticker",
    "video_note",
    "voice_note",
];
pub const CAPTIONLESS_KINDS: [&str; 2] = ["sticker", "video_note"];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Video,
    Document,
    Audio,
    Animation,
    Sticker,
    VideoNote,
    VoiceNote,
}
impl MediaKind {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "photo" => Self::Photo,
            "video" => Self::Video,
            "document" => Self::Document,
            "audio" => Self::Audio,
            "animation" => Self::Animation,
            "sticker" => Self::Sticker,
            "video_note" => Self::VideoNote,
            "voice_note" => Self::VoiceNote,
            _ => {
                return Err(Error::MediaKind {
                    kind: value.to_owned(),
                });
            }
        })
    }
    pub fn as_str(self) -> &'static str {
        MEDIA_KINDS[match self {
            Self::Photo => 0,
            Self::Video => 1,
            Self::Document => 2,
            Self::Audio => 3,
            Self::Animation => 4,
            Self::Sticker => 5,
            Self::VideoNote => 6,
            Self::VoiceNote => 7,
        }]
    }
    pub fn captionless(self) -> bool {
        matches!(self, Self::Sticker | Self::VideoNote)
    }
}
pub fn guess_mime(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}
pub fn resolve_kind(
    given: Option<MediaKind>,
    mime: &str,
    caption: &str,
    header: &[u8],
) -> Result<MediaKind> {
    resolve_kind_named("", given, mime, caption, header)
}
pub(crate) fn resolve_kind_named(
    name: &str,
    given: Option<MediaKind>,
    mime: &str,
    caption: &str,
    header: &[u8],
) -> Result<MediaKind> {
    let kind = given.unwrap_or_else(|| {
        if mime == "image/gif" {
            MediaKind::Animation
        } else if mime.starts_with("image/") {
            MediaKind::Photo
        } else if mime.starts_with("video/") {
            MediaKind::Video
        } else if mime.starts_with("audio/") {
            if crate::ogg::looks_like_voice(header) == Some(true) {
                MediaKind::VoiceNote
            } else {
                MediaKind::Audio
            }
        } else {
            MediaKind::Document
        }
    });
    if mime != "application/octet-stream" {
        let valid = match kind {
            MediaKind::Document => true,
            MediaKind::Photo | MediaKind::Sticker => mime.starts_with("image/"),
            MediaKind::Video | MediaKind::VideoNote => mime.starts_with("video/"),
            MediaKind::Animation => mime == "image/gif" || mime.starts_with("video/"),
            MediaKind::Audio => mime.starts_with("audio/"),
            MediaKind::VoiceNote => matches!(mime, "audio/ogg" | "audio/opus" | "audio/x-opus"),
        };
        if !valid {
            return Err(Error::MediaKindMismatch {
                file_name: name.to_owned(),
                kind: kind.as_str(),
            });
        }
    }
    if !caption.is_empty() && kind.captionless() {
        return Err(Error::MediaCaption {
            kind: kind.as_str(),
        });
    }
    Ok(kind)
}
