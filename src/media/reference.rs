use crate::{
    Deserializable, Serializable, enums,
    error::{Error, Result},
    files::{FileKey, encrypted_size},
    types,
};
use grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
#[derive(Clone, PartialEq)]
pub struct MediaReference {
    pub chat_id: i32,
    pub file_id: i64,
    pub access_hash: i64,
    pub dc_id: i32,
    pub key_fingerprint: i32,
    media: Vec<u8>,
}
impl std::fmt::Debug for MediaReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaReference")
            .field("chat_id", &self.chat_id)
            .field("file_id", &self.file_id)
            .finish()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    chat_id: i32,
    file_id: i64,
    access_hash: i64,
    dc_id: i32,
    key_fingerprint: i32,
    media: String,
}
pub struct FileParts {
    pub key: FileKey,
    pub size: u64,
    pub mime_type: Option<String>,
}
impl MediaReference {
    pub fn new(
        chat_id: i32,
        file: &tl::types::EncryptedFile,
        media: &enums::DecryptedMessageMedia,
    ) -> Result<Self> {
        let reference = Self {
            chat_id,
            file_id: file.id,
            access_hash: file.access_hash,
            dc_id: file.dc_id,
            key_fingerprint: file.key_fingerprint,
            media: media.to_bytes(),
        };
        reference.validate()?;
        Ok(reference)
    }
    pub fn from_parts(
        chat_id: i32,
        file_id: i64,
        access_hash: i64,
        dc_id: i32,
        key_fingerprint: i32,
        media: Vec<u8>,
    ) -> Result<Self> {
        let reference = Self {
            chat_id,
            file_id,
            access_hash,
            dc_id,
            key_fingerprint,
            media,
        };
        reference.validate()?;
        Ok(reference)
    }
    pub fn decoded(&self) -> Result<enums::DecryptedMessageMedia> {
        crate::wire::validate(&self.media, "DecryptedMessageMedia").map_err(|_| Error::Media)?;
        let media =
            enums::DecryptedMessageMedia::from_bytes(&self.media).map_err(|_| Error::Media)?;
        file_parts(&media)?;
        Ok(media)
    }
    pub fn validate(&self) -> Result<FileParts> {
        let parts = file_parts(&self.decoded()?)?;
        parts.key.verify(self.key_fingerprint)?;
        Ok(parts)
    }
    pub fn size(&self) -> Result<u64> {
        Ok(self.validate()?.size)
    }
    pub fn mime_type(&self) -> Result<Option<String>> {
        Ok(self.validate()?.mime_type)
    }
    pub fn media_bytes(&self) -> &[u8] {
        &self.media
    }
    pub fn to_json(&self) -> Result<serde_json::Value> {
        self.validate()?;
        serde_json::to_value(Record {
            version: 1,
            chat_id: self.chat_id,
            file_id: self.file_id,
            access_hash: self.access_hash,
            dc_id: self.dc_id,
            key_fingerprint: self.key_fingerprint,
            media: hex::encode(&self.media),
        })
        .map_err(|_| Error::Media)
    }
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or(Error::MediaField { field: "record" })?;
        fn field<T: serde::de::DeserializeOwned>(
            object: &serde_json::Map<String, serde_json::Value>,
            name: &'static str,
        ) -> Result<T> {
            serde_json::from_value(
                object
                    .get(name)
                    .cloned()
                    .ok_or(Error::MediaField { field: name })?,
            )
            .map_err(|_| Error::MediaField { field: name })
        }
        if object.keys().any(|name| {
            !matches!(
                name.as_str(),
                "version"
                    | "chat_id"
                    | "file_id"
                    | "access_hash"
                    | "dc_id"
                    | "key_fingerprint"
                    | "media"
            )
        }) {
            return Err(Error::MediaField { field: "record" });
        }
        let record = Record {
            version: field(object, "version")?,
            chat_id: field(object, "chat_id")?,
            file_id: field(object, "file_id")?,
            access_hash: field(object, "access_hash")?,
            dc_id: field(object, "dc_id")?,
            key_fingerprint: field(object, "key_fingerprint")?,
            media: field(object, "media")?,
        };
        if record.version != 1 {
            return Err(Error::MediaField { field: "version" });
        }
        let reference = Self {
            chat_id: record.chat_id,
            file_id: record.file_id,
            access_hash: record.access_hash,
            dc_id: record.dc_id,
            key_fingerprint: record.key_fingerprint,
            media: hex::decode(record.media).map_err(|_| Error::MediaField { field: "media" })?,
        };
        reference
            .decoded()
            .map_err(|_| Error::MediaField { field: "media" })?;
        reference.validate().map_err(|_| Error::MediaField {
            field: "key_fingerprint",
        })?;
        Ok(reference)
    }
    pub fn server_handle(&self) -> tl::enums::InputEncryptedFile {
        tl::types::InputEncryptedFile {
            id: self.file_id,
            access_hash: self.access_hash,
        }
        .into()
    }
}
impl grammers_client::media::Downloadable for MediaReference {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputEncryptedFileLocation {
                id: self.file_id,
                access_hash: self.access_hash,
            }
            .into(),
        )
    }
    fn size(&self) -> Option<usize> {
        encrypted_size(self.size().ok()?).ok()?.try_into().ok()
    }
    fn dc_id(&self) -> Option<i32> {
        Some(self.dc_id)
    }
}
pub fn file_parts(media: &enums::DecryptedMessageMedia) -> Result<FileParts> {
    macro_rules! parts {
        ($m:expr,$size:expr,$mime:expr) => {
            FileParts {
                key: FileKey::new(&$m.key, &$m.iv)?,
                size: u64::try_from($size).map_err(|_| Error::Media)?,
                mime_type: $mime,
            }
        };
    }
    use enums::DecryptedMessageMedia as M;
    Ok(match media {
        M::Document(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::DocumentLegacy7afe8ae2(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::DocumentLegacyb095434b(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::Photo(m) => parts!(m, m.size, Some("image/jpeg".to_owned())),
        M::PhotoLegacy32798a8c(m) => parts!(m, m.size, None),
        M::Video(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::VideoLegacy524a415d(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::VideoLegacy4cee6ef3(m) => parts!(m, m.size, None),
        M::Audio(m) => parts!(m, m.size, Some(m.mime_type.clone())),
        M::AudioLegacy6080758f(m) => parts!(m, m.size, None),
        _ => return Err(Error::Media),
    })
}
pub fn forwarded_media(
    reference: &MediaReference,
    layer: i32,
    caption: &str,
) -> Result<enums::DecryptedMessageMedia> {
    reference.validate()?;
    let mut media = reference.decoded()?;
    let attributes = match &media {
        enums::DecryptedMessageMedia::Document(m) => Some(&m.attributes),
        enums::DecryptedMessageMedia::DocumentLegacy7afe8ae2(m) => Some(&m.attributes),
        _ => None,
    };
    if !caption.is_empty() {
        for attribute in attributes.into_iter().flatten() {
            if matches!(attribute, enums::DocumentAttribute::Sticker(_)) {
                return Err(Error::MediaCaption { kind: "sticker" });
            }
            if matches!(attribute,enums::DocumentAttribute::Video(v) if v.round_message) {
                return Err(Error::MediaCaption { kind: "video_note" });
            }
        }
    }
    use enums::DecryptedMessageMedia as M;
    match &mut media {
        M::Document(m) => m.caption = caption.to_owned(),
        M::DocumentLegacy7afe8ae2(m) => m.caption = caption.to_owned(),
        M::Photo(m) => m.caption = caption.to_owned(),
        M::Video(m) => m.caption = caption.to_owned(),
        _ => {}
    }
    if layer < 143 {
        if reference.size()? >= 1u64 << 31 {
            return Err(Error::Media);
        }
        if let M::Document(m) = media {
            return Ok(types::DecryptedMessageMediaDocumentLegacy7afe8ae2 {
                thumb: m.thumb,
                thumb_w: m.thumb_w,
                thumb_h: m.thumb_h,
                mime_type: m.mime_type,
                size: i32::try_from(m.size).map_err(|_| Error::Media)?,
                key: m.key,
                iv: m.iv,
                attributes: m.attributes,
                caption: m.caption,
            }
            .into());
        }
    }
    Ok(media)
}
