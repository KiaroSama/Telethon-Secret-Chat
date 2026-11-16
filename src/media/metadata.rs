use super::MediaKind;
use crate::{
    enums,
    error::{Error, Result},
    files::FileKey,
    types,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub duration: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub thumbnail: Option<Vec<u8>>,
    pub thumbnail_size: Option<(i32, i32)>,
    pub waveform: Option<Vec<u8>>,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub sticker_alt: Option<String>,
}
impl Metadata {
    pub fn validate(&self, kind: MediaKind) -> Result<()> {
        use MediaKind::*;
        let invalid = |field| Error::MediaField { field };
        for (field, value, allowed) in [
            (
                "duration",
                self.duration,
                matches!(kind, Video | VideoNote | Audio | VoiceNote),
            ),
            (
                "width",
                self.width,
                matches!(kind, Photo | Video | VideoNote | Animation | Sticker),
            ),
            (
                "height",
                self.height,
                matches!(kind, Photo | Video | VideoNote | Animation | Sticker),
            ),
        ] {
            if value.is_some_and(|n| n < 0 || !allowed) {
                return Err(invalid(field));
            }
        }
        if self
            .waveform
            .as_ref()
            .is_some_and(|w| kind != VoiceNote || w.is_empty() || w.len() > 63)
        {
            return Err(invalid("waveform"));
        }
        if self.title.is_some() && kind != Audio {
            return Err(invalid("title"));
        }
        if self.performer.is_some() && kind != Audio {
            return Err(invalid("performer"));
        }
        if self.sticker_alt.is_some() && kind != Sticker {
            return Err(invalid("sticker_alt"));
        }
        match (&self.thumbnail, self.thumbnail_size) {
            (None, None) => {}
            (Some(bytes), Some((w, h))) => {
                if bytes.is_empty()
                    || bytes.len() >= 200 * 1024
                    || !(1..=320).contains(&w)
                    || !(1..=320).contains(&h)
                {
                    return Err(invalid("thumbnail"));
                }
                let jpeg = bytes.starts_with(&[255, 216, 255]);
                let webp =
                    bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice());
                if !(jpeg || (kind == Sticker && webp)) {
                    return Err(invalid("thumbnail"));
                }
            }
            _ => return Err(invalid("thumbnail_size")),
        }
        Ok(())
    }
    pub fn from_json(value: serde_json::Value, kind: MediaKind) -> Result<Self> {
        let metadata: Self =
            serde_json::from_value(value).map_err(|_| Error::MediaField { field: "metadata" })?;
        metadata.validate(kind)?;
        Ok(metadata)
    }
}
pub fn attributes_for(
    kind: MediaKind,
    name: &str,
    metadata: &Metadata,
) -> Result<Vec<enums::DocumentAttribute>> {
    metadata.validate(kind)?;
    let mut attributes = vec![
        types::DocumentAttributeFilename {
            file_name: name.to_owned(),
        }
        .into(),
    ];
    let (w, h) = (metadata.width.unwrap_or(0), metadata.height.unwrap_or(0));
    match kind {
        MediaKind::Photo => attributes.push(types::DocumentAttributeImageSize { w, h }.into()),
        MediaKind::Video | MediaKind::VideoNote => attributes.push(
            types::DocumentAttributeVideo {
                round_message: kind == MediaKind::VideoNote,
                duration: metadata.duration.unwrap_or(0),
                w,
                h,
            }
            .into(),
        ),
        MediaKind::Audio | MediaKind::VoiceNote => attributes.push(
            types::DocumentAttributeAudio {
                voice: kind == MediaKind::VoiceNote,
                duration: metadata.duration.unwrap_or(0),
                title: metadata.title.clone(),
                performer: metadata.performer.clone(),
                waveform: metadata.waveform.clone(),
            }
            .into(),
        ),
        MediaKind::Animation => {
            attributes.push(enums::DocumentAttribute::Animated);
            if w != 0 || h != 0 {
                attributes.push(types::DocumentAttributeImageSize { w, h }.into());
            }
        }
        MediaKind::Sticker => {
            attributes.push(
                types::DocumentAttributeSticker {
                    alt: metadata.sticker_alt.clone().unwrap_or_default(),
                    stickerset: enums::InputStickerSet::Empty,
                }
                .into(),
            );
            if w != 0 || h != 0 {
                attributes.push(types::DocumentAttributeImageSize { w, h }.into());
            }
        }
        MediaKind::Document => {}
    }
    Ok(attributes)
}
/// Inputs for one validated secret-document constructor.
pub struct DocumentMediaInput<'a> {
    pub kind: MediaKind,
    pub name: &'a str,
    pub mime: &'a str,
    pub size: u64,
    pub layer: i32,
    pub key: &'a FileKey,
    pub caption: &'a str,
    pub metadata: &'a Metadata,
}
pub fn document_media(input: DocumentMediaInput<'_>) -> Result<enums::DecryptedMessageMedia> {
    let DocumentMediaInput {
        kind,
        name,
        mime,
        size,
        layer,
        key,
        caption,
        metadata,
    } = input;
    if kind.captionless() && !caption.is_empty() {
        return Err(Error::MediaCaption {
            kind: kind.as_str(),
        });
    }
    let attributes = attributes_for(kind, name, metadata)?;
    let (thumb_w, thumb_h) = metadata.thumbnail_size.unwrap_or((0, 0));
    let thumb = metadata.thumbnail.clone().unwrap_or_default();
    if layer < 143 {
        let size = i32::try_from(size).map_err(|_| Error::Media)?;
        Ok(types::DecryptedMessageMediaDocumentLegacy7afe8ae2 {
            thumb,
            thumb_w,
            thumb_h,
            mime_type: mime.to_owned(),
            size,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes,
            caption: caption.to_owned(),
        }
        .into())
    } else {
        let size = i64::try_from(size).map_err(|_| Error::Media)?;
        Ok(types::DecryptedMessageMediaDocument {
            thumb,
            thumb_w,
            thumb_h,
            mime_type: mime.to_owned(),
            size,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes,
            caption: caption.to_owned(),
        }
        .into())
    }
}
