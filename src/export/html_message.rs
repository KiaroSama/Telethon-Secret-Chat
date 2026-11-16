//! Desktop service lines, default/joined bubbles, forwarding and message parts.
use super::{
    error::{ExportError, Result},
    format::{DateContext, format_date_time},
    html_media::prepare_media_data_with_countries,
    html_rich::render_rich_message,
    html_rich_media::NativeRichMedia,
    html_safe::safe_message_href,
    html_service::service_text,
    html_text::*,
    model::*,
    model_actions::Action,
    model_dialogs::DialogInfo,
    model_media::MediaContent,
    model_message::*,
    thumbnail::{ThumbnailOptions, write_image_thumb},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageInfoType {
    #[default]
    Service,
    Default,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageInfo {
    pub id: i64,
    pub r#type: MessageInfoType,
    pub from_id: i64,
    pub via_bot_id: i64,
    pub date: i64,
    pub forwarded_from_id: i64,
    pub forwarded_from_name: String,
    pub forwarded: bool,
    pub show_forwarded_as_original: bool,
    pub forwarded_date: i64,
}
pub type MessageMixin = WrapBase;
pub fn write_userpic_thumb(
    base: &Path,
    large_path: &str,
    userpic: &UserpicData,
    postfix: &str,
) -> String {
    let size = userpic.pixel_size.saturating_mul(2);
    write_image_thumb(
        base,
        large_path,
        |_| (size, size),
        &ThumbnailOptions {
            postfix,
            ..Default::default()
        },
    )
    .0
}
fn button_onclick(button: &HistoryMessageMarkupButton) -> String {
    let data = String::from_utf8_lossy(&button.data);
    let content = format!(
        "{}{}Type: {}",
        if button.data.is_empty() {
            String::new()
        } else {
            format!("Data: {data} | ")
        },
        if button.forward_text.is_empty() {
            String::new()
        } else {
            format!("Forward text: {} | ", button.forward_text)
        },
        button.type_to_string()
    );
    format!(
        "return ShowTextCopied('{}');",
        content.replace('\\', "\\\\").replace('\'', "\\'")
    )
}
fn append(output: &mut Vec<u8>, value: impl AsRef<[u8]>) {
    output.extend_from_slice(value.as_ref());
}
impl WrapBase {
    pub fn push_service_message(
        &mut self,
        id: i64,
        dialog: &DialogInfo,
        base: &Path,
        serialized: &[u8],
        photo: Option<&Photo>,
    ) -> Result<Vec<u8>> {
        let mut result = self
            .push_tag(
                "div",
                &attrs([
                    ("class", "message service"),
                    ("id", &format!("message{id}")),
                ]),
            )
            .into_bytes();
        append(&mut result, self.push_div("body details", ""));
        append(&mut result, serialized);
        append(&mut result, self.pop_tag()?);
        if let Some(photo) = photo {
            let mut userpic = UserpicData {
                color_index: dialog.color_index,
                first_name: dialog.name.clone(),
                last_name: dialog.last_name.clone(),
                pixel_size: 60,
                large_link: photo.image.file.relative_path.clone(),
                ..Default::default()
            };
            userpic.image_link = write_userpic_thumb(base, &userpic.large_link, &userpic, "_thumb");
            append(&mut result, self.push_div("userpic_wrap", ""));
            append(&mut result, self.push_userpic(&userpic)?);
            append(&mut result, self.pop_tag()?);
        }
        append(&mut result, self.pop_tag()?);
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn push_message(
        &mut self,
        message: &Message,
        previous: Option<&MessageInfo>,
        dialog: &DialogInfo,
        base: &Path,
        peers: &PeersMap,
        domain: &str,
        mut link: impl FnMut(i64, &str) -> String,
        dates: &DateContext,
    ) -> Result<(MessageInfo, Vec<u8>)> {
        let mut info = MessageInfo {
            id: message.id,
            from_id: message.from_id,
            via_bot_id: message.via_bot_id,
            date: message.date,
            forwarded_from_id: message.forwarded_from_id,
            forwarded_from_name: message.forwarded_from_name.clone(),
            forwarded_date: message.forwarded_date,
            forwarded: message.forwarded,
            show_forwarded_as_original: message.show_forwarded_as_original,
            ..Default::default()
        };
        if matches!(
            message.media.content,
            Some(MediaContent::UnsupportedMedia(_))
        ) {
            return Ok((info,self.push_service_message(message.id,dialog,base,b"This message is not supported by this version of Telegram Desktop. Please update the application.",None)?));
        }
        let service = service_text(
            message,
            dialog,
            peers,
            domain,
            &self.base,
            |text| link(message.reply_to_msg_id, text),
            dates,
        )?;
        if !service.is_empty() {
            let photo = match &message.action.content {
                Some(Action::ChatEditPhoto(data)) => Some(&data.photo),
                Some(Action::SuggestProfilePhoto(data)) => Some(&data.photo),
                _ => None,
            };
            return Ok((
                info,
                self.push_service_message(message.id, dialog, base, &service, photo)?,
            ));
        }
        info.r#type = MessageInfoType::Default;
        Ok((
            info,
            self.push_default(message, previous, base, peers, domain, &mut link, dates)?,
        ))
    }
    fn userpics(&self, message: &Message, peers: &PeersMap) -> (UserpicData, UserpicData) {
        let mut forwarded = UserpicData::default();
        if message.forwarded {
            forwarded.color_index = if message.forwarded_from_id != 0 {
                peer_id_color_index(message.forwarded_from_id)
            } else {
                [0, 7, 4, 1, 6, 3, 5][((message.id as u64) % 7) as usize]
            };
            forwarded.pixel_size = 42;
            if message.forwarded_from_id != 0 {
                fill_userpic_names_from_peer(
                    &mut forwarded,
                    &peers.peer(message.forwarded_from_id),
                );
            } else {
                fill_userpic_names_from_full(&mut forwarded, &message.forwarded_from_name);
            }
        }
        if message.show_forwarded_as_original {
            return (forwarded.clone(), forwarded);
        }
        let mut userpic = UserpicData {
            color_index: peer_id_color_index(message.from_id),
            pixel_size: 42,
            ..Default::default()
        };
        fill_userpic_names_from_peer(&mut userpic, &peers.peer(message.from_id));
        (userpic, forwarded)
    }
    #[allow(clippy::too_many_arguments)]
    fn push_default(
        &mut self,
        message: &Message,
        previous: Option<&MessageInfo>,
        base: &Path,
        peers: &PeersMap,
        domain: &str,
        mut link: impl FnMut(i64, &str) -> String,
        dates: &DateContext,
    ) -> Result<Vec<u8>> {
        let wrap = self.message_needs_wrap(message, previous, dates)?;
        let show_forwarded = message.forwarded && !message.show_forwarded_as_original;
        let (userpic, forwarded_userpic) = self.userpics(message, peers);
        let via = if message.via_bot_id != 0 {
            serialize_string(&peers.user(message.via_bot_id).username)
        } else {
            String::new()
        };
        let class = format!(
            "message default clearfix{}",
            if wrap { "" } else { " joined" }
        );
        let mut block = self
            .push_tag(
                "div",
                &attrs([("class", &class), ("id", &format!("message{}", message.id))]),
            )
            .into_bytes();
        if wrap {
            append(&mut block, self.push_div("pull_left userpic_wrap", ""));
            append(&mut block, self.push_userpic(&userpic)?);
            append(&mut block, self.pop_tag()?);
        }
        append(&mut block, self.push_div("body", ""));
        append(
            &mut block,
            self.push_tag(
                "div",
                &attrs([
                    ("class", "pull_right date details"),
                    (
                        "title",
                        &format_date_time(message.date, true, ".", ":", " ", dates)?,
                    ),
                ]),
            ),
        );
        append(&mut block, format_time_text(message.date, dates)?);
        append(&mut block, self.pop_tag()?);
        if wrap {
            append(&mut block, self.push_div("from_name", ""));
            append(
                &mut block,
                serialize_string(&compose_name(&userpic, "Deleted Account")),
            );
            if !via.is_empty() && (!message.forwarded || message.show_forwarded_as_original) {
                append(&mut block, format!(" via @{via}"));
            }
            append(&mut block, self.pop_tag()?);
        }
        if show_forwarded {
            let forwarded_wrap = self.forwarded_needs_wrap(message, previous, dates)?;
            if forwarded_wrap {
                append(
                    &mut block,
                    self.push_div("pull_left forwarded userpic_wrap", ""),
                );
                append(&mut block, self.push_userpic(&forwarded_userpic)?);
                append(&mut block, self.pop_tag()?);
            }
            append(&mut block, self.push_div("forwarded body", ""));
            if forwarded_wrap {
                append(&mut block, self.push_div("from_name", ""));
                append(
                    &mut block,
                    serialize_string(&compose_name(&forwarded_userpic, "Deleted Account")),
                );
                if !via.is_empty() {
                    append(&mut block, format!(" via @{via}"));
                }
                append(
                    &mut block,
                    self.push_tag(
                        "span",
                        &attrs([
                            ("class", "date details"),
                            (
                                "title",
                                &format_date_time(
                                    message.forwarded_date,
                                    true,
                                    ".",
                                    ":",
                                    " ",
                                    dates,
                                )?,
                            ),
                            ("inline", ""),
                        ]),
                    ),
                );
                append(
                    &mut block,
                    format!(
                        " {}",
                        format_date_time(message.forwarded_date, false, ".", ":", " ", dates)?
                    ),
                );
                append(&mut block, self.pop_tag()?);
                append(&mut block, self.pop_tag()?);
            }
        }
        if message.reply_to_msg_id != 0 {
            append(&mut block, self.push_div("reply_to details", ""));
            if message.reply_to_peer_id != 0 {
                append(&mut block, "In reply to a message in another chat");
            } else {
                append(
                    &mut block,
                    format!(
                        "In reply to {}",
                        link(message.reply_to_msg_id, "this message")
                    ),
                );
            }
            append(&mut block, self.pop_tag()?);
        }
        append(
            &mut block,
            self.push_media(message, base, peers, domain, &mut link, dates)?,
        );
        if let Some(rich) = &message.rich_message {
            let mut media = NativeRichMedia::new(&self.base, base);
            append(
                &mut block,
                render_rich_message(
                    &mut self.context,
                    rich,
                    message.id,
                    domain,
                    &self.base,
                    &mut media,
                    dates,
                )?,
            );
        } else {
            let text = format_text(&message.text, domain, &self.base);
            if !text.is_empty() {
                append(&mut block, self.push_div("text", ""));
                append(&mut block, text);
                append(&mut block, self.pop_tag()?);
            }
        }
        if !message.inline_button_rows.is_empty() {
            append(&mut block, self.push_inline_buttons(message)?);
        }
        if !message.signature.is_empty() {
            append(&mut block, self.push_div("signature details", ""));
            append(&mut block, serialize_string(&message.signature));
            append(&mut block, self.pop_tag()?);
        }
        if show_forwarded {
            append(&mut block, self.pop_tag()?);
        }
        if !message.reactions.is_empty() {
            append(&mut block, self.push_reactions(&message.reactions, peers)?);
        }
        append(&mut block, self.pop_tag()?);
        append(&mut block, self.pop_tag()?);
        Ok(block)
    }
    pub fn push_inline_buttons(&mut self, message: &Message) -> Result<String> {
        let mut block = self.push_tag("table", &attrs([("class", "bot_buttons_table")]));
        block += &self.push_tag("tbody", &BTreeMap::new());
        for row in &message.inline_button_rows {
            block += &self.push_tag("tr", &BTreeMap::new());
            block += &self.push_tag("td", &attrs([("class", "bot_button_row")]));
            for (index, button) in row.iter().enumerate() {
                let url = button.r#type == MarkupButtonType::Url;
                let href = if url {
                    safe_message_href(
                        std::str::from_utf8(&button.data)
                            .map_err(|_| ExportError::InvalidRecord)?,
                    )
                    .unwrap_or_default()
                } else {
                    String::new()
                };
                let onclick = if url {
                    String::new()
                } else {
                    button_onclick(button)
                };
                let mut attributes = BTreeMap::new();
                for (name, value) in [("href", href), ("onclick", onclick)] {
                    attributes
                        .entry(if value.is_empty() {
                            String::new()
                        } else {
                            name.into()
                        })
                        .or_insert(value);
                }
                block += &self.push_div("bot_button", "");
                block += &self.push_tag("a", &attributes);
                block += &self.push_tag("div", &BTreeMap::new());
                block += &serialize_string(&button.text);
                block += &self.pop_tag()?;
                block += &self.pop_tag()?;
                block += &self.pop_tag()?;
                if index != row.len() - 1 {
                    block += &self.push_div("bot_button_column_separator", "");
                    block += &self.pop_tag()?;
                }
            }
            block += &self.pop_tag()?;
            block += &self.pop_tag()?;
        }
        block += &self.pop_tag()?;
        Ok(block + &self.pop_tag()?)
    }
    pub fn push_reactions(&mut self, reactions: &[Reaction], peers: &PeersMap) -> Result<String> {
        let mut block = self.push_tag("span", &attrs([("class", "reactions")]));
        for reaction in reactions {
            let mut class = "reaction".to_owned();
            if reaction
                .recent
                .iter()
                .any(|r| peers.peer(r.peer_id).user().is_some_and(|u| u.is_self))
            {
                class += " active";
            }
            if reaction.r#type == ReactionType::Paid {
                class += " paid";
            }
            block += &self.push_tag("span", &attrs([("class", &class)]));
            block += &self.push_tag("span", &attrs([("class", "emoji")]));
            match reaction.r#type {
                ReactionType::Emoji => block += &serialize_string(&reaction.emoji),
                ReactionType::CustomEmoji => {
                    block += &format_custom_emoji(&reaction.document_id, "👋", &self.base)
                }
                ReactionType::Paid => block += "⭐",
                ReactionType::Empty => {}
            }
            block += &self.pop_tag()?;
            if !reaction.recent.is_empty() {
                block += &self.push_tag("span", &attrs([("class", "userpics")]));
                for recent in &reaction.recent {
                    let peer = peers.peer(recent.peer_id);
                    let user = peer.user();
                    block += &self.push_userpic(&UserpicData {
                        color_index: peer.color_index(),
                        pixel_size: 20,
                        first_name: user.map_or_else(|| peer.name(), |u| u.info.first_name.clone()),
                        last_name: user.map_or_else(String::new, |u| u.info.last_name.clone()),
                        tooltip: peer.name(),
                        ..Default::default()
                    })?;
                }
                block += &self.pop_tag()?;
            }
            if reaction.recent.is_empty() || reaction.count > reaction.recent.len() as i64 {
                block += &self.push_tag("span", &attrs([("class", "count")]));
                block += &reaction.count.to_string();
                block += &self.pop_tag()?;
            }
            block += &self.pop_tag()?;
        }
        Ok(block + &self.pop_tag()?)
    }
    pub fn message_needs_wrap(
        &self,
        message: &Message,
        previous: Option<&MessageInfo>,
        dates: &DateContext,
    ) -> Result<bool> {
        let Some(previous) = previous.filter(|p| p.r#type == MessageInfoType::Default) else {
            return Ok(true);
        };
        if message.from_id == 0
            || previous.from_id != message.from_id
            || message.via_bot_id != previous.via_bot_id
            || dates.datetime(previous.date)?.date() != dates.datetime(message.date)?.date()
            || message.forwarded != previous.forwarded
            || message.show_forwarded_as_original != previous.show_forwarded_as_original
            || message.forwarded_from_id != previous.forwarded_from_id
            || message.forwarded_from_name != previous.forwarded_from_name
        {
            return Ok(true);
        }
        let limit = if message.forwarded_from_id != 0 || !message.forwarded_from_name.is_empty() {
            1
        } else {
            900
        };
        Ok(message.date.abs_diff(previous.date) > limit)
    }
    pub fn forwarded_needs_wrap(
        &self,
        message: &Message,
        previous: Option<&MessageInfo>,
        dates: &DateContext,
    ) -> Result<bool> {
        if self.message_needs_wrap(message, previous, dates)? {
            return Ok(true);
        }
        let Some(previous) = previous else {
            return Ok(true);
        };
        Ok(message.forwarded_from_id == 0
            || message.forwarded_from_id != previous.forwarded_from_id
            || !peer_is_user(message.forwarded_from_id)
            || message.forwarded_date.abs_diff(previous.forwarded_date) > 900)
    }
    pub fn push_media(
        &mut self,
        message: &Message,
        base: &Path,
        peers: &PeersMap,
        domain: &str,
        mut link: impl FnMut(i64, &str) -> String,
        dates: &DateContext,
    ) -> Result<Vec<u8>> {
        let data =
            prepare_media_data_with_countries(message, peers, domain, &self.context.countries)?;
        if !data.classes.is_empty() {
            return Ok(self.push_generic_media(&data)?.into_bytes());
        }
        Ok(match &message.media.content {
            Some(MediaContent::Document(content)) => {
                if content.is_sticker {
                    self.push_sticker_media(content, base)?.into_bytes()
                } else if content.is_animated {
                    self.push_animated_media(content, base)?.into_bytes()
                } else if content.is_video_file {
                    self.push_video_file_media(content, base)?.into_bytes()
                } else {
                    return Err(ExportError::InvalidRecord);
                }
            }
            Some(MediaContent::Photo(content)) => {
                self.push_photo_media(content, base)?.into_bytes()
            }
            Some(MediaContent::Poll(content)) => {
                self.push_poll(content, domain, &self.base.clone())?
            }
            Some(MediaContent::TodoList(content)) => {
                self.push_todo_list(content, domain, &self.base.clone())?
            }
            Some(MediaContent::GiveawayStart(content)) => self
                .push_giveaway_start(peers, content, dates)?
                .into_bytes(),
            Some(MediaContent::GiveawayResults(content)) => self
                .push_giveaway_results(peers, content, &mut link)?
                .into_bytes(),
            _ => vec![],
        })
    }
}
