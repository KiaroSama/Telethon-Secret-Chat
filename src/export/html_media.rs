//! Desktop media cards, thumbnails and rich file presentation.
use super::{
    error::Result,
    format::{
        fill_amount_and_currency, format_duration, format_file_size, format_image_size_text,
        format_phone_number,
    },
    html_rich_support::*,
    html_safe::is_global_link,
    html_text::{PeersMap, WrapBase, attrs, serialize_string},
    model::*,
    model_actions::{Action, PhoneCallState},
    model_media::MediaContent,
    model_message::Message,
    thumbnail::{ThumbnailOptions, write_image_thumb},
};
use std::path::Path;
pub type Size = (u32, u32);
fn scaled_keep_aspect(size: Size, width: u32, height: u32) -> Size {
    if size.0 == 0 || size.1 == 0 {
        return (width, height);
    }
    let rw = u64::from(height) * u64::from(size.0) / u64::from(size.1);
    if rw <= u64::from(width) {
        (rw as u32, height)
    } else {
        (
            width,
            (u64::from(width) * u64::from(size.1) / u64::from(size.0)) as u32,
        )
    }
}
/// `min_width`/`min_height` are retained in the API; Desktop checks its fixed80px minima.
pub fn calculate_thumb_size(
    max_width: u32,
    max_height: u32,
    _min_width: u32,
    _min_height: u32,
    retina: bool,
) -> impl Fn(Size) -> Size {
    move |large| {
        let multiplier = if retina { 2 } else { 1 };
        let small = if large.0.saturating_mul(multiplier) > max_width
            || large.1.saturating_mul(multiplier) > max_height
        {
            scaled_keep_aspect(large, max_width, max_height)
        } else {
            large
        };
        let result = (small.0 & !1, small.1 & !1);
        if result.0 < 80 || result.1 < 80 {
            (0, 0)
        } else {
            result
        }
    }
}
pub fn image_readable(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .is_some()
}
pub fn prepare_audio_media_data(data: &Document) -> Result<MediaData> {
    let mut result = MediaData {
        title: if !data.song_performer.is_empty() && !data.song_title.is_empty() {
            format!("{} – {}", data.song_performer, data.song_title)
        } else if data.name.is_empty() {
            "Audio file".into()
        } else {
            data.name.clone()
        },
        status: format_duration(u64::from(data.duration)),
        classes: "media_audio_file".into(),
        link: data.file.relative_path.clone(),
        description: no_file_description(data.file.skip_reason.clone())?.into(),
        ..Default::default()
    };
    if data.file.relative_path.is_empty() {
        result.status += &format!(", {}", format_file_size(data.file.size));
    }
    Ok(result)
}
pub fn prepare_file_media_data(data: &Document) -> Result<MediaData> {
    Ok(MediaData {
        title: if data.name.is_empty() {
            "File".into()
        } else {
            data.name.clone()
        },
        status: format_file_size(data.file.size),
        classes: "media_file".into(),
        link: data.file.relative_path.clone(),
        description: no_file_description(data.file.skip_reason.clone())?.into(),
        ..Default::default()
    })
}
fn maps_link(latitude: &str, longitude: &str) -> String {
    format!("https://maps.google.com/maps?q={latitude},{longitude}&ll={latitude},{longitude}&z=16")
}
pub fn prepare_media_data(message: &Message, peers: &PeersMap, domain: &str) -> Result<MediaData> {
    prepare_media_data_with_countries(message, peers, domain, &[])
}
pub fn prepare_media_data_with_countries(
    message: &Message,
    peers: &PeersMap,
    domain: &str,
    countries: &[super::format::CountryInfo],
) -> Result<MediaData> {
    let mut result = MediaData::default();
    if let Some(Action::PhoneCall(call)) = &message.action.content {
        result.classes = "media_call".into();
        result.title = peers
            .peer(if message.out {
                message.peer_id
            } else {
                message.self_id
            })
            .name();
        result.status = match call.state {
            PhoneCallState::Invitation => "Invitation",
            PhoneCallState::Active => "Ongoing",
            PhoneCallState::Missed if message.out => "Cancelled",
            _ if message.out => "Outgoing",
            PhoneCallState::Missed => "Missed",
            PhoneCallState::Busy => "Declined",
            _ => "Incoming",
        }
        .into();
        if call.duration > 0 {
            result.classes += " success";
            result.status += &format!(" ({} seconds)", call.duration);
        }
        return Ok(result);
    }
    use MediaContent as C;
    match &message.media.content {
        Some(C::Photo(data)) => {
            if message.media.ttl != 0 {
                result.title = "Self-destructing photo".into();
                result.status = if data.id != 0 {
                    "Please view it on your mobile"
                } else {
                    "Expired"
                }
                .into();
                result.classes = "media_photo".into();
            }
        }
        Some(C::Document(data)) => {
            if message.media.ttl != 0 {
                return Ok(MediaData {
                    title: "Self-destructing video".into(),
                    status: if data.id != 0 {
                        "Please view it on your mobile"
                    } else {
                        "Expired"
                    }
                    .into(),
                    classes: "media_video".into(),
                    ..Default::default()
                });
            }
            result.link = data.file.relative_path.clone();
            result.description = no_file_description(data.file.skip_reason.clone())?.into();
            if data.is_sticker {
            } else if data.is_video_message || data.is_voice_message {
                result.title = if data.is_video_message {
                    "Video message"
                } else {
                    "Voice message"
                }
                .into();
                result.status = format_duration(u64::from(data.duration));
                if data.file.relative_path.is_empty() {
                    result.status += &format!(", {}", format_file_size(data.file.size));
                }
                if data.is_video_message {
                    result.thumb = data.thumb.file.relative_path.clone();
                    result.classes = "media_video".into();
                } else {
                    result.classes = "media_voice_message".into();
                }
            } else if data.is_animated || data.is_video_file {
            } else if data.is_audio_file {
                return prepare_audio_media_data(data);
            } else {
                return prepare_file_media_data(data);
            }
        }
        Some(C::SharedContact(data)) => {
            result.title = format!("{} {}", data.info.first_name, data.info.last_name);
            result.classes = "media_contact".into();
            result.status = format_phone_number(&data.info.phone_number, countries);
            if !data.vcard.content.is_empty() {
                result.status += " - vCard";
                result.link = data.vcard.relative_path.clone();
            }
        }
        Some(C::GeoPoint(data)) => {
            let live = message.media.ttl != 0;
            result.classes = if live {
                "media_live_location"
            } else {
                "media_location"
            }
            .into();
            result.title = if live { "Live location" } else { "Location" }.into();
            if data.valid {
                let lat = format!("{:.6}", data.latitude);
                let lon = format!("{:.6}", data.longitude);
                result.status = format!("{lat}, {lon}");
                result.link = maps_link(&lat, &lon);
            }
        }
        Some(C::Venue(data)) => {
            result.classes = "media_venue".into();
            result.title = data.title.clone();
            result.description = data.address.clone();
            if data.point.valid {
                result.link = maps_link(
                    &format!("{:.6}", data.point.latitude),
                    &format!("{:.6}", data.point.longitude),
                );
            }
        }
        Some(C::Game(data)) => {
            result.classes = "media_game".into();
            result.title = data.title.clone();
            result.description = data.description.clone();
            if data.bot_id != 0 && !data.short_name.is_empty() {
                let bot = peers.user(data.bot_id);
                if bot.is_bot && !bot.username.is_empty() {
                    let link = format!("{domain}{}?game={}", bot.username, data.short_name);
                    result.link = link.clone();
                    result.status = link;
                }
            }
        }
        Some(C::Invoice(data)) => {
            result.classes = "media_invoice".into();
            result.title = data.title.clone();
            result.description = data.description.clone();
            result.status = fill_amount_and_currency(data.amount, &data.currency, false);
        }
        Some(C::PaidMedia(data)) => {
            result.classes = "media_invoice".into();
            result.status = fill_amount_and_currency(data.stars, "XTR", false);
        }
        _ => {}
    }
    Ok(result)
}
impl WrapBase {
    pub fn push_generic_media(&mut self, data: &MediaData) -> Result<String> {
        let mut result = self.push_div("media_wrap clearfix", "");
        if data.link.is_empty() {
            result += &self.push_div(&format!("media clearfix pull_left {}", data.classes), "");
        } else {
            let href = if is_global_link(&data.link) {
                data.link.clone()
            } else {
                self.relative_path(&data.link)
            };
            result += &self.push_tag(
                "a",
                &attrs([
                    (
                        "class",
                        &format!("media clearfix pull_left block_link {}", data.classes),
                    ),
                    ("href", &href),
                ]),
            );
        }
        if data.thumb.is_empty() {
            result += &self.push_div("fill pull_left", "");
            result += &self.pop_tag()?;
        } else {
            result += &self.push_tag(
                "img",
                &attrs([
                    ("class", "thumb pull_left"),
                    ("src", &self.relative_path(&data.thumb)),
                    ("empty", ""),
                ]),
            );
        }
        result += &self.push_div("body", "");
        for (value, class) in [
            (&data.title, "title bold"),
            (&data.description, "description"),
            (&data.status, "status details"),
        ] {
            if !value.is_empty() {
                result += &self.push_div(class, "");
                result += &serialize_string(value);
                result += &self.pop_tag()?;
            }
        }
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    fn open_sized(&mut self, class: &str, href: &str) -> String {
        let mut result = self.push_div("media_wrap clearfix", "");
        result += &self.push_tag(
            "a",
            &attrs([("class", class), ("href", &self.relative_path(href))]),
        );
        result
    }
    fn image_tag(&mut self, class: &str, size: Size, source: &str) -> String {
        self.push_tag(
            "img",
            &attrs([
                ("class", class),
                (
                    "style",
                    &format!("width: {}px; height: {}px", size.0 / 2, size.1 / 2),
                ),
                ("src", &self.relative_path(source)),
                ("empty", ""),
            ]),
        )
    }
    pub fn push_sticker_media(&mut self, data: &Document, base: &Path) -> Result<String> {
        let options = ThumbnailOptions {
            image_format: Some("PNG"),
            ..Default::default()
        };
        let (thumb, size) = write_image_thumb(
            base,
            &data.file.relative_path,
            calculate_thumb_size(384, 384, 80, 80, false),
            &options,
        );
        if thumb.is_empty() {
            let mut generic = MediaData {
                title: "Sticker".into(),
                status: data.sticker_emoji.clone(),
                description: no_file_description(data.file.skip_reason.clone())?.into(),
                classes: "media_photo".into(),
                ..Default::default()
            };
            if data.file.relative_path.is_empty() {
                if !generic.status.is_empty() {
                    generic.status += ", ";
                }
                generic.status += &format_file_size(data.file.size);
            } else {
                generic.link = data.file.relative_path.clone();
            }
            return self.push_generic_media(&generic);
        }
        let mut result =
            self.open_sized("sticker_wrap clearfix pull_left", &data.file.relative_path);
        result += &self.image_tag("sticker", size, &thumb);
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    fn video_thumb_size(&self, data: &Document) -> Size {
        calculate_thumb_size(520, 520, 80, 80, true)((data.width, data.height))
    }
    pub fn push_animated_media(&mut self, data: &Document, _base: &Path) -> Result<String> {
        let size = self.video_thumb_size(data);
        if data.thumb.file.relative_path.is_empty()
            || data.file.relative_path.is_empty()
            || size.0 == 0
            || size.1 == 0
        {
            return self.push_generic_media(&MediaData {
                title: "Animation".into(),
                status: format_file_size(data.file.size),
                link: data.file.relative_path.clone(),
                description: no_file_description(data.file.skip_reason.clone())?.into(),
                classes: "media_video".into(),
                ..Default::default()
            });
        }
        let mut result =
            self.open_sized("animated_wrap clearfix pull_left", &data.file.relative_path);
        result += &self.push_div("video_play_bg", "");
        result += &self.push_div("gif_play", "");
        result += "GIF";
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        result += &self.image_tag("animated", size, &data.thumb.file.relative_path);
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    pub fn push_video_file_media(&mut self, data: &Document, _base: &Path) -> Result<String> {
        let size = self.video_thumb_size(data);
        if data.thumb.file.relative_path.is_empty()
            || data.file.relative_path.is_empty()
            || size.0 == 0
            || size.1 == 0
        {
            let mut generic = MediaData {
                title: "Video file".into(),
                status: format_duration(u64::from(data.duration)),
                description: no_file_description(data.file.skip_reason.clone())?.into(),
                classes: "media_video".into(),
                ..Default::default()
            };
            if data.file.relative_path.is_empty() {
                generic.status += &format!(", {}", format_file_size(data.file.size));
            } else {
                generic.link = data.file.relative_path.clone();
            }
            return self.push_generic_media(&generic);
        }
        let mut result = self.open_sized(
            "video_file_wrap clearfix pull_left",
            &data.file.relative_path,
        );
        result += &self.push_div("video_play_bg", "");
        result += &self.push_div("video_play", "");
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        result += &self.push_div("video_duration", "");
        result += &format_duration(u64::from(data.duration));
        result += &self.pop_tag()?;
        result += &self.image_tag("video_file", size, &data.thumb.file.relative_path);
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    pub fn push_photo_media(&mut self, data: &Photo, base: &Path) -> Result<String> {
        let (thumb, size) = write_image_thumb(
            base,
            &data.image.file.relative_path,
            calculate_thumb_size(520, 520, 80, 80, false),
            &ThumbnailOptions::default(),
        );
        if thumb.is_empty() {
            let image = &data.image;
            let mut generic = MediaData {
                title: "Photo".into(),
                status: format_image_size_text(image.width, image.height),
                description: no_file_description(image.file.skip_reason.clone())?.into(),
                classes: "media_photo".into(),
                ..Default::default()
            };
            if image.file.relative_path.is_empty() {
                generic.status += &format!(", {}", format_file_size(image.file.size));
            } else {
                generic.link = image.file.relative_path.clone();
            }
            return self.push_generic_media(&generic);
        }
        let mut result = self.open_sized(
            "photo_wrap clearfix pull_left",
            &data.image.file.relative_path,
        );
        result += &self.image_tag("photo", size, &thumb);
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    pub fn push_rich_photo_media(&mut self, data: Option<&Photo>, base: &Path) -> Result<String> {
        self.push_photo_media(&rich_photo_presentation(data), base)
    }
    pub fn push_rich_video_media(
        &mut self,
        data: Option<&Document>,
        base: &Path,
    ) -> Result<String> {
        let mut presentation = rich_document_presentation(data);
        let thumb = &presentation.thumb.file.relative_path;
        if !thumb.is_empty() && !image_readable(&base.join(thumb)) {
            presentation.thumb.file.relative_path.clear();
        }
        self.push_video_file_media(&presentation, base)
    }
    pub fn push_rich_audio_media(&mut self, data: Option<&Document>) -> Result<String> {
        self.push_generic_media(&prepare_audio_media_data(&rich_document_presentation(
            data,
        ))?)
    }
    pub fn push_rich_file_media(&mut self, data: Option<&Document>) -> Result<String> {
        self.push_generic_media(&prepare_file_media_data(&rich_document_presentation(data))?)
    }
    pub fn push_rich_reference_media(
        &mut self,
        data: &mut MediaData,
        photo: Option<&Photo>,
        base: &Path,
    ) -> Result<String> {
        let path = rich_photo_presentation(photo).image.file.relative_path;
        data.link = path.clone();
        data.thumb = if path.is_empty() {
            String::new()
        } else {
            write_image_thumb(
                base,
                &path,
                |_| (96, 96),
                &ThumbnailOptions {
                    postfix: "_rich_card_thumb",
                    ..Default::default()
                },
            )
            .0
        };
        self.push_generic_media(data)
    }
}
