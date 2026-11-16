//! Desktop photo/document parsing and stable generated file names.
use super::{
    error::Result,
    files::{file_name_from_user_string, mime_first_glob},
    format::DateContext,
    model::*,
};
use serde_json::{Value, json};
pub fn bytes(v: &Value) -> Vec<u8> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|v| v as u8)
                .collect()
        })
        .unwrap_or_default()
}
pub fn refresh_file_reference(to: &mut FileLocation, source: &FileLocation) -> bool {
    if to.dc_id != source.dc_id || kind(&to.data) != kind(&source.data) {
        return false;
    }
    let supported = matches!(
        tl_kind(&to.data),
        "InputDocumentFileLocation" | "InputPhotoFileLocation"
    );
    if !supported
        || to.data.get("id") != source.data.get("id")
        || to.data.get("thumb_size") != source.data.get("thumb_size")
    {
        return false;
    }
    to.dc_id = source.dc_id;
    to.data = source.data.clone();
    true
}
pub fn tl_kind(v: &Value) -> &str {
    let name = kind(v);
    if name.is_empty() {
        return name;
    }
    name
}
fn is(v: &Value, expected: &str) -> bool {
    let name = kind(v);
    name == expected
        || name
            .get(..1)
            .is_some_and(|c| c.eq_ignore_ascii_case(&expected[..1]))
            && name.get(1..) == expected.get(1..)
}
fn image_dimensions(value: &Value) -> Option<(u32, u32)> {
    // Telegram dimensions are positive TL ints, including when read from JSON.
    let width = i32::try_from(integer(value, "w")).ok()?;
    let height = i32::try_from(integer(value, "h")).ok()?;
    (width > 0 && height > 0).then_some((width as u32, height as u32))
}
pub fn parse_max_image(photo: &Value, suggested: &str) -> Image {
    let mut result = Image::default();
    result.file.suggested_path = suggested.into();
    let mut max_area = 0u64;
    for size in array(photo, "sizes") {
        if ["PhotoSizeEmpty", "PhotoStrippedSize", "PhotoPathSize"]
            .iter()
            .any(|k| is(size, k))
        {
            continue;
        }
        let Some((w, h)) = image_dimensions(size) else {
            continue;
        };
        let area = u64::from(w) * u64::from(h);
        if area <= max_area {
            continue;
        }
        if is(size, "PhotoSizeProgressive") && array(size, "sizes").is_empty() {
            continue;
        }
        result.width = w;
        result.height = h;
        result.file.location = FileLocation {
            dc_id: integer(photo, "dc_id") as i32,
            data: json!({"_":"InputPhotoFileLocation","id":photo["id"],"access_hash":photo["access_hash"],"file_reference":photo["file_reference"],"thumb_size":size["type"]}),
        };
        if is(size, "PhotoCachedSize") {
            result.file.content = bytes(&size["bytes"]);
            result.file.size = result.file.content.len() as u64;
        } else if is(size, "PhotoSizeProgressive") {
            let Some(last) = array(size, "sizes").last() else {
                continue;
            };
            result.file.content.clear();
            result.file.size = last.as_u64().unwrap_or(0);
        } else {
            result.file.content.clear();
            result.file.size = integer(size, "size").max(0) as u64;
        }
        max_area = area;
    }
    result
}
pub fn parse_photo(value: &Value, suggested: &str) -> Photo {
    let mut result = Photo::default();
    if is(value, "Photo") {
        result.id = integer(value, "id");
        result.date = integer(value, "date");
        result.image = parse_max_image(value, suggested);
    } else if is(value, "PhotoEmpty") {
        result.id = integer(value, "id");
    }
    result
}
pub fn parse_attributes(document: &mut Document, attributes: &[Value]) {
    for value in attributes {
        let name = kind(value);
        let name = if name.starts_with('d') {
            let mut n = name.to_owned();
            n.replace_range(..1, "D");
            n
        } else {
            name.to_owned()
        };
        match name.as_str() {
            "DocumentAttributeImageSize" => {
                document.width = integer(value, "w").max(0) as u32;
                document.height = integer(value, "h").max(0) as u32;
            }
            "DocumentAttributeAnimated" => document.is_animated = true,
            "DocumentAttributeSticker" | "DocumentAttributeCustomEmoji" => {
                document.is_sticker = true;
                document.sticker_emoji = string(value, "alt").into();
            }
            "DocumentAttributeVideo" => {
                if boolean(value, "round_message") {
                    document.is_video_message = true;
                } else {
                    document.is_video_file = true;
                }
                document.width = integer(value, "w").max(0) as u32;
                document.height = integer(value, "h").max(0) as u32;
                document.duration = value
                    .get("duration")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0) as u32;
            }
            "DocumentAttributeAudio" => {
                if boolean(value, "voice") {
                    document.is_voice_message = true;
                } else {
                    document.is_audio_file = true;
                }
                document.song_performer = string(value, "performer").into();
                document.song_title = string(value, "title").into();
                document.duration = integer(value, "duration").max(0) as u32;
            }
            "DocumentAttributeFilename" => document.name = string(value, "file_name").into(),
            _ => {}
        }
    }
}
pub fn compute_document_name(
    context: &mut ParseMediaContext,
    data: &Document,
    date: i64,
    dates: &DateContext,
) -> Result<String> {
    if !data.name.is_empty() {
        return Ok(data.name.clone());
    }
    let pattern = mime_first_glob(&data.mime);
    let extension = pattern.replace('*', "");
    let date_part = prepare_file_name_date_part(date, dates)?;
    if data.is_voice_message {
        context.audios += 1;
        Ok(format!(
            "audio_{}{date_part}{}",
            context.audios,
            if data.mime.eq_ignore_ascii_case("audio/mp3") {
                ".mp3"
            } else {
                ".ogg"
            }
        ))
    } else if data.is_video_file {
        context.videos += 1;
        Ok(format!(
            "video_{}{date_part}{}",
            context.videos,
            if pattern.is_empty() {
                ".mov"
            } else {
                &extension
            }
        ))
    } else {
        context.files += 1;
        Ok(format!(
            "file_{}{date_part}{}",
            context.files,
            if pattern.is_empty() {
                ".unknown"
            } else {
                &extension
            }
        ))
    }
}
pub fn document_folder(data: &Document) -> &'static str {
    if data.is_video_file {
        "video_files"
    } else if data.is_animated {
        "animations"
    } else if data.is_sticker {
        "stickers"
    } else if data.is_voice_message {
        "voice_messages"
    } else if data.is_video_message {
        "round_video_messages"
    } else {
        "files"
    }
}
pub fn parse_document_thumb(document: &Value, path: &str) -> Image {
    let best = array(document, "thumbs")
        .iter()
        .filter(|size| {
            !["PhotoSizeEmpty", "PhotoStrippedSize", "PhotoPathSize"]
                .iter()
                .any(|kind| is(size, kind))
                && (!is(size, "PhotoSizeProgressive") || !array(size, "sizes").is_empty())
        })
        .filter_map(|size| image_dimensions(size).map(|(width, height)| (size, width, height)))
        .max_by_key(|(_, width, height)| u64::from(*width) * u64::from(*height));
    let Some((best, width, height)) = best else {
        return Image::default();
    };
    let mut result = Image {
        width,
        height,
        ..Default::default()
    };
    result.file.location = FileLocation {
        dc_id: integer(document, "dc_id") as i32,
        data: json!({"_":"InputDocumentFileLocation","id":document["id"],"access_hash":document["access_hash"],"file_reference":document["file_reference"],"thumb_size":best["type"]}),
    };
    if is(best, "PhotoCachedSize") {
        result.file.content = bytes(&best["bytes"]);
        result.file.size = result.file.content.len() as u64;
    } else if is(best, "PhotoSizeProgressive") {
        let Some(last) = array(best, "sizes").last() else {
            return Image::default();
        };
        result.file.size = last.as_u64().unwrap_or(0);
    } else {
        result.file.size = integer(best, "size").max(0) as u64;
    }
    result.file.suggested_path = format!("{path}_thumb.jpg");
    result
}
pub fn parse_document(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<Document> {
    let mut result = Document::default();
    if is(data, "Document") {
        result.id = integer(data, "id");
        result.date = integer(data, "date");
        result.mime = string(data, "mime_type").into();
        parse_attributes(&mut result, array(data, "attributes"));
        result.file.size = integer(data, "size").max(0) as u64;
        result.file.location = FileLocation {
            dc_id: integer(data, "dc_id") as i32,
            data: json!({"_":"InputDocumentFileLocation","id":data["id"],"access_hash":data["access_hash"],"file_reference":data["file_reference"],"thumb_size":""}),
        };
        result.file.suggested_path = format!(
            "{folder}{}/{}",
            document_folder(&result),
            file_name_from_user_string(&compute_document_name(context, &result, date, dates)?)
        );
        result.thumb = parse_document_thumb(data, &result.file.suggested_path);
    } else if is(data, "DocumentEmpty") {
        result.id = integer(data, "id");
    }
    Ok(result)
}
