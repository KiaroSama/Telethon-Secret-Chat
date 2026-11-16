//! Rich export file availability and exact Desktop metadata ordering.
use super::{json_serialize::*, model::*};
pub fn append_file_availability(
    values: &mut Pairs,
    file: Option<&File>,
    path_key: &str,
    reason_key: &str,
) {
    if let Some(file) = file {
        let reason = match file.skip_reason {
            SkipReason::Unavailable => Some("unavailable"),
            SkipReason::FileType => Some("file_type"),
            SkipReason::FileSize => Some("file_size"),
            SkipReason::DateLimits => Some("date_limits"),
            SkipReason::None => None,
        };
        if let Some(reason) = reason {
            values.push((reason_key.into(), serialize_string(reason)));
            return;
        }
        if !file.relative_path.is_empty() {
            values.push((path_key.into(), serialize_string(&file.relative_path)));
            return;
        }
    }
    values.push((reason_key.into(), serialize_string("unavailable")));
}
pub fn append_photo_metadata(values: &mut Pairs, photo: Option<&Photo>, keys: [&str; 5]) {
    let [path_key, reason_key, size_key, width_key, height_key] = keys;
    append_file_availability(values, photo.map(|p| &p.image.file), path_key, reason_key);
    if let Some(photo) = photo {
        values.push((size_key.into(), number(photo.image.file.size)));
        if photo.image.width != 0 && photo.image.height != 0 {
            values.push((width_key.into(), number(photo.image.width)));
            values.push((height_key.into(), number(photo.image.height)));
        }
    }
}
pub fn document_media_type_name(document: &Document) -> &'static str {
    if document.is_sticker {
        "sticker"
    } else if document.is_video_message {
        "video_message"
    } else if document.is_voice_message {
        "voice_message"
    } else if document.is_animated {
        "animation"
    } else if document.is_video_file {
        "video_file"
    } else if document.is_audio_file {
        "audio_file"
    } else {
        ""
    }
}
pub fn append_document_metadata(values: &mut Pairs, document: Option<&Document>, dimensions: bool) {
    append_file_availability(
        values,
        document.map(|d| &d.file),
        "file",
        "file_skip_reason",
    );
    let Some(d) = document else {
        return;
    };
    let mut string = |key: &str, text: &str| {
        if !text.is_empty() {
            values.push((key.into(), serialize_string(text)));
        }
    };
    string("file_name", &d.name);
    values.push(("file_size".into(), number(d.file.size)));
    if d.thumb.width > 0 {
        append_file_availability(
            values,
            Some(&d.thumb.file),
            "thumbnail",
            "thumbnail_skip_reason",
        );
        values.push(("thumbnail_file_size".into(), number(d.thumb.file.size)));
    }
    let media_type = document_media_type_name(d);
    if !media_type.is_empty() {
        values.push(("media_type".into(), serialize_string(media_type)));
    }
    if d.is_sticker && !d.sticker_emoji.is_empty() {
        values.push(("sticker_emoji".into(), serialize_string(&d.sticker_emoji)));
    } else if media_type == "audio_file" {
        if !d.song_performer.is_empty() {
            values.push(("performer".into(), serialize_string(&d.song_performer)));
        }
        if !d.song_title.is_empty() {
            values.push(("title".into(), serialize_string(&d.song_title)));
        }
    }
    if !d.mime.is_empty() {
        values.push(("mime_type".into(), serialize_string(&d.mime)));
    }
    if d.duration != 0 {
        values.push(("duration_seconds".into(), number(d.duration)));
    }
    if dimensions && d.width != 0 && d.height != 0 {
        values.push(("width".into(), number(d.width)));
        values.push(("height".into(), number(d.height)));
    }
}
