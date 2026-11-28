//! Stateless rich HTML metadata, availability, target and presentation helpers.
use super::{
    error::{ExportError, Result},
    html_text::attrs,
    json_rich::{base64_url, enum_name},
    model::*,
    model_rich::*,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaData {
    pub title: String,
    pub description: String,
    pub status: String,
    pub classes: String,
    pub thumb: String,
    pub link: String,
}
pub fn no_file_description(reason: SkipReason) -> Result<&'static str> {
    Ok(match reason {
        SkipReason::Unavailable => "Unavailable, please try again later.",
        SkipReason::FileSize => "Exceeds maximum size, change data exporting settings to download.",
        SkipReason::FileType => "Not included, change data exporting settings to download.",
        SkipReason::None => "",
        SkipReason::DateLimits => return Err(ExportError::InvalidRecord),
    })
}
pub fn rich_file_presentation(file: Option<&File>) -> File {
    let mut result = file.cloned().unwrap_or_default();
    result.skip_reason = rich_file_availability(file);
    result
}
/// Normalize availability without copying embedded file content.
fn rich_file_availability(file: Option<&File>) -> SkipReason {
    match file {
        Some(file) if !file.relative_path.is_empty() => SkipReason::None,
        Some(file)
            if matches!(
                file.skip_reason,
                SkipReason::FileType | SkipReason::FileSize
            ) =>
        {
            file.skip_reason.clone()
        }
        _ => SkipReason::Unavailable,
    }
}
pub fn rich_photo_presentation(photo: Option<&Photo>) -> Photo {
    let mut result = photo.cloned().unwrap_or_default();
    result.image.file = rich_file_presentation(photo.map(|p| &p.image.file));
    result
}
pub fn rich_document_presentation(document: Option<&Document>) -> Document {
    let mut result = document.cloned().unwrap_or_default();
    result.file = rich_file_presentation(document.map(|d| &d.file));
    result.thumb.file = rich_file_presentation(document.map(|d| &d.thumb.file));
    result
}
pub fn rich_file_description(file: Option<&File>) -> Result<&'static str> {
    no_file_description(rich_file_availability(file))
}
pub fn rich_block_attributes(classes: &str, kind: &str) -> BTreeMap<String, String> {
    attrs([
        ("class", &format!("rich_block {classes}")),
        ("data-rich-kind", kind),
    ])
}
pub fn rich_heading_tag(level: i64) -> String {
    format!("h{}", if (1..=6).contains(&level) { level } else { 6 })
}
pub fn rich_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
pub fn rich_button_metadata(value: &str) -> String {
    let mut result = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            result.push(char::from(b));
        } else {
            result += &format!("%{b:02X}");
        }
    }
    result
}
pub fn rich_button_peer_types(types: &[InlineButtonPeerType]) -> String {
    rich_button_metadata(&format!(
        "[{}]",
        types
            .iter()
            .map(|t| format!("\"{}\"", enum_name(t)))
            .collect::<Vec<_>>()
            .join(",")
    ))
}
pub fn rich_button_attributes(
    action: &InlineButtonAction,
    style: Option<RichButtonStyle>,
    inline: bool,
) -> BTreeMap<String, String> {
    let style_name = enum_name(style.unwrap_or_default());
    let type_name = enum_name(action.r#type);
    let mut result = attrs([
        (
            "class",
            &format!(
                "rich_button {} rich_button_style_{style_name} rich_button_action_{type_name}",
                if inline {
                    "rich_button_inline"
                } else {
                    "rich_button_block"
                }
            ),
        ),
        ("data-button-type", &type_name),
        ("inline", ""),
    ]);
    if style.is_some() {
        result.insert("data-button-style".into(), style_name);
    }
    use InlineButtonActionType as A;
    match action.r#type {
        A::Url | A::Auth | A::WebView => {
            result.insert("data-button-url".into(), rich_button_metadata(&action.url));
            if action.r#type == A::Auth {
                if let Some(text) = &action.forward_text {
                    result.insert(
                        "data-button-forward-text".into(),
                        rich_button_metadata(text),
                    );
                }
                result.insert("data-button-id".into(), action.button_id.to_string());
            }
        }
        A::Callback | A::CallbackWithPassword => {
            result.insert(
                "data-callback-data-base64".into(),
                String::from_utf8(base64_url(&action.callback_data)).unwrap_or_default(),
            );
            result.insert(
                "data-button-requires-password".into(),
                rich_bool(action.requires_password).into(),
            );
        }
        A::SwitchInline | A::SwitchInlineSame => {
            result.insert(
                "data-button-query".into(),
                rich_button_metadata(&action.query),
            );
            result.insert(
                "data-button-same-peer".into(),
                rich_bool(action.same_peer).into(),
            );
            if let Some(types) = &action.peer_types {
                result.insert(
                    "data-button-peer-types".into(),
                    rich_button_peer_types(types),
                );
            }
        }
        A::UserProfile => {
            result.insert("data-button-user-id".into(), action.user_id.to_string());
        }
        A::CopyText => {
            result.insert(
                "data-copy-text".into(),
                rich_button_metadata(&action.copy_text),
            );
        }
        A::Disabled => {
            result.insert("aria-disabled".into(), "true".into());
        }
        _ => {}
    }
    result
}
pub fn rich_percent_value(value: i64, total: i64) -> String {
    format!("{:.3}%", value as f64 * 100.0 / total as f64)
}
pub fn rich_aspect_ratio_style(width: i64, height: i64) -> String {
    format!("aspect-ratio: {width} / {height}")
}
pub fn rich_text_has_output(text: &RichText) -> bool {
    match text.r#type {
        RichTextType::Empty => false,
        RichTextType::Plain => !text.text.is_empty(),
        RichTextType::Concat => text.children.iter().any(rich_text_has_output),
        _ => true,
    }
}
pub fn rich_caption_has_output(caption: &RichCaption) -> bool {
    rich_text_has_output(&caption.text) || rich_text_has_output(&caption.credit)
}
pub fn rich_channel_source_name(source: RichChannelSource) -> String {
    enum_name(source)
}
pub fn rich_channel_status(channel: &RichChannel) -> String {
    let label = match channel.source {
        RichChannelSource::ChatEmpty | RichChannelSource::Chat => "Chat",
        RichChannelSource::ChatForbidden => "Chat unavailable",
        RichChannelSource::Channel => "Channel",
        RichChannelSource::ChannelForbidden => "Channel unavailable",
        RichChannelSource::Community => "Community",
        RichChannelSource::CommunityForbidden => "Community unavailable",
    };
    if let Some(name) = &channel.username
        && !name.is_empty()
    {
        return format!("@{name}, {label}");
    }
    label.into()
}
pub fn rich_map_point_source_name(source: RichMapPointSource) -> String {
    enum_name(source)
}
pub fn rich_map_coordinates(point: &RichMapPoint) -> Option<(String, String)> {
    if matches!(
        point.source,
        RichMapPointSource::GeoPointEmpty | RichMapPointSource::InputGeoPointEmpty
    ) || !point.latitude.is_finite()
        || !point.longitude.is_finite()
        || !(-90.0..=90.0).contains(&point.latitude)
        || !(-180.0..=180.0).contains(&point.longitude)
    {
        return None;
    }
    Some((
        format!("{:.6}", point.latitude),
        format!("{:.6}", point.longitude),
    ))
}
pub fn plain_target(text: &RichText) -> Option<String> {
    fn append(result: &mut String, text: &RichText) -> bool {
        use RichTextType as T;
        match text.r#type {
            T::Empty => true,
            T::Plain | T::CustomEmoji => {
                if text.text.len() > 4096 - result.len() {
                    false
                } else {
                    result.push_str(&text.text);
                    true
                }
            }
            T::InlineImage | T::Math | T::Button => false,
            _ => text.children.iter().all(|c| append(result, c)),
        }
    }
    let mut result = String::new();
    append(&mut result, text).then_some(result)
}
