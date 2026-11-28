//! Desktop messages, reactions, inline buttons and message ordering.
use super::{
    error::Result,
    format::DateContext,
    model::*,
    model_actions::{Action, ServiceAction},
    model_document::bytes,
    model_media::{Media, MediaContent},
    model_peers::{Peers, parse_peers_lists, type_is},
    model_rich::RichMessage,
    parse_actions::parse_service_action,
    parse_media::parse_media,
    parse_rich::parse_rich_message,
    settings::Settings,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionType {
    #[default]
    Empty,
    Emoji,
    CustomEmoji,
    Paid,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentReaction {
    pub peer_id: i64,
    pub date: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub r#type: ReactionType,
    pub emoji: String,
    pub document_id: String,
    pub count: i64,
    pub recent: Vec<RecentReaction>,
}
impl Reaction {
    pub fn type_to_string(&self) -> &'static str {
        match self.r#type {
            ReactionType::Empty => "empty",
            ReactionType::Emoji => "emoji",
            ReactionType::CustomEmoji => "custom_emoji",
            ReactionType::Paid => "paid",
        }
    }
    pub fn id(&self) -> String {
        self.type_to_string().to_owned()
            + match self.r#type {
                ReactionType::Emoji => &self.emoji,
                ReactionType::CustomEmoji => &self.document_id,
                _ => "",
            }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MessageId {
    pub channel_id: i64,
    pub msg_id: i64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkupButtonType {
    #[default]
    Default,
    Url,
    Callback,
    CallbackWithPassword,
    RequestPhone,
    RequestLocation,
    RequestPoll,
    RequestPeer,
    SwitchInline,
    SwitchInlineSame,
    Game,
    Buy,
    Auth,
    UserProfile,
    WebView,
    SimpleWebView,
    CopyText,
    Disabled,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryMessageMarkupButton {
    pub r#type: MarkupButtonType,
    pub text: String,
    pub data: Vec<u8>,
    pub forward_text: String,
    pub button_id: i64,
}
impl HistoryMessageMarkupButton {
    pub fn type_to_string(&self) -> &'static str {
        match self.r#type {
            MarkupButtonType::Default => "default",
            MarkupButtonType::Url => "url",
            MarkupButtonType::Callback => "callback",
            MarkupButtonType::CallbackWithPassword => "callback_with_password",
            MarkupButtonType::RequestPhone => "request_phone",
            MarkupButtonType::RequestLocation => "request_location",
            MarkupButtonType::RequestPoll => "request_poll",
            MarkupButtonType::RequestPeer => "request_peer",
            MarkupButtonType::SwitchInline => "switch_inline",
            MarkupButtonType::SwitchInlineSame => "switch_inline_same",
            MarkupButtonType::Game => "game",
            MarkupButtonType::Buy => "buy",
            MarkupButtonType::Auth => "auth",
            MarkupButtonType::UserProfile => "user_profile",
            MarkupButtonType::WebView => "web_view",
            MarkupButtonType::SimpleWebView => "simple_web_view",
            MarkupButtonType::CopyText => "copy_text",
            MarkupButtonType::Disabled => "disabled",
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    pub id: i64,
    pub date: i64,
    pub edited: i64,
    pub from_id: i64,
    pub peer_id: i64,
    pub self_id: i64,
    pub forwarded_from_id: i64,
    pub forwarded_from_name: String,
    pub forwarded_date: i64,
    pub forwarded: bool,
    pub show_forwarded_as_original: bool,
    pub saved_from_chat_id: i64,
    pub signature: String,
    pub via_bot_id: i64,
    pub reply_to_msg_id: i64,
    pub reply_to_peer_id: i64,
    pub text: Vec<TextPart>,
    pub reactions: Vec<Reaction>,
    pub media: Media,
    pub action: ServiceAction,
    pub out: bool,
    pub inline_button_rows: Vec<Vec<HistoryMessageMarkupButton>>,
    pub rich_message: Option<RichMessage>,
}
impl Message {
    pub fn file(&self) -> File {
        match &self.action.content {
            Some(Action::ChatEditPhoto(a)) => a.photo.image.file.clone(),
            Some(Action::SuggestProfilePhoto(a)) => a.photo.image.file.clone(),
            _ => self.media.file(),
        }
    }
    pub fn file_mut(&mut self) -> Option<&mut File> {
        match &mut self.action.content {
            Some(Action::ChatEditPhoto(a)) => Some(&mut a.photo.image.file),
            Some(Action::SuggestProfilePhoto(a)) => Some(&mut a.photo.image.file),
            _ => self.media.file_mut(),
        }
    }
    pub fn thumb(&self) -> Image {
        self.media.thumb()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileOrigin {
    pub split: i64,
    pub peer: Value,
    pub message_id: i64,
    pub story_id: i64,
    pub custom_emoji_id: i64,
    pub rich_message: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessagesSlice {
    pub list: Vec<Message>,
    pub peers: Peers,
}
pub fn parse_reaction(reaction: &Value) -> Reaction {
    if type_is(reaction, "ReactionEmoji") {
        Reaction {
            r#type: ReactionType::Emoji,
            emoji: string(reaction, "emoticon").into(),
            ..Default::default()
        }
    } else if type_is(reaction, "ReactionCustomEmoji") {
        Reaction {
            r#type: ReactionType::CustomEmoji,
            document_id: integer(reaction, "document_id").to_string(),
            ..Default::default()
        }
    } else if type_is(reaction, "ReactionPaid") {
        Reaction {
            r#type: ReactionType::Paid,
            ..Default::default()
        }
    } else {
        Reaction::default()
    }
}
pub fn parse_reactions(data: &Value) -> Vec<Reaction> {
    let mut reactions: Vec<Reaction> = vec![];
    for single in array(data, "results") {
        let mut reaction = parse_reaction(&single["reaction"]);
        reaction.count = integer(single, "count");
        if !reactions.iter().any(|r| r.id() == reaction.id()) {
            reactions.push(reaction);
        }
    }
    for recent in array(data, "recent_reactions") {
        let reaction = parse_reaction(&recent["reaction"]);
        let id = reaction.id();
        let index = match reactions.iter().position(|r| r.id() == id) {
            Some(i) => i,
            None => {
                reactions.push(reaction);
                reactions.len() - 1
            }
        };
        reactions[index].recent.push(RecentReaction {
            peer_id: parse_peer_id(&recent["peer_id"]),
            date: integer(recent, "date"),
        });
    }
    reactions
}
fn markup_button(button: &Value) -> Option<HistoryMessageMarkupButton> {
    let data = &button["type"];
    let mut result = HistoryMessageMarkupButton {
        text: string(button, "text").into(),
        ..Default::default()
    };
    let name = kind(data)
        .trim_start_matches("InlineButtonType")
        .trim_start_matches("inlineButtonType");
    match name {
        "Url" | "WebView" | "UrlAuth" => {
            result.r#type = match name {
                "Url" => MarkupButtonType::Url,
                "WebView" => MarkupButtonType::WebView,
                _ => MarkupButtonType::Auth,
            };
            result.data = string(data, "url").as_bytes().to_vec();
            if name == "UrlAuth" {
                result.forward_text = string(data, "fwd_text").into();
                result.button_id = integer(data, "button_id");
            }
        }
        "Callback" => {
            result.r#type = if boolean(data, "requires_password") {
                MarkupButtonType::CallbackWithPassword
            } else {
                MarkupButtonType::Callback
            };
            result.data = bytes(&data["data"]);
        }
        "Game" => result.r#type = MarkupButtonType::Game,
        "Buy" => result.r#type = MarkupButtonType::Buy,
        "SwitchInline" => {
            result.r#type = if boolean(data, "same_peer") {
                MarkupButtonType::SwitchInlineSame
            } else {
                MarkupButtonType::SwitchInline
            };
            result.data = string(data, "query").as_bytes().to_vec();
        }
        "UserProfile" => {
            result.r#type = MarkupButtonType::UserProfile;
            result.data = integer(data, "user_id").to_string().into_bytes();
        }
        "Copy" => {
            result.r#type = MarkupButtonType::CopyText;
            result.data = string(data, "copy_text").as_bytes().to_vec();
        }
        "Disabled" => result.r#type = MarkupButtonType::Disabled,
        _ => return None,
    }
    Some(result)
}
pub fn button_rows_from_tl(data: &Value) -> Vec<Vec<HistoryMessageMarkupButton>> {
    array(data, "rows")
        .iter()
        .map(|row| {
            array(row, "buttons")
                .iter()
                .filter_map(markup_button)
                .collect::<Vec<_>>()
        })
        .filter(|r| !r.is_empty())
        .collect()
}
fn reply_header(result: &mut Message, reply: &Value, drop_same_peer: bool) {
    if !type_is(reply, "MessageReplyHeader") || reply["reply_to_msg_id"].is_null() {
        return;
    }
    result.reply_to_msg_id = integer(reply, "reply_to_msg_id");
    result.reply_to_peer_id = parse_peer_id(&reply["reply_to_peer_id"]);
    if drop_same_peer && result.reply_to_peer_id == result.peer_id {
        result.reply_to_peer_id = 0;
    }
}
pub fn parse_message(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    dates: &DateContext,
) -> Result<Message> {
    let mut result = Message {
        id: integer(data, "id"),
        ..Default::default()
    };
    if type_is(data, "MessageEmpty") {
        return Ok(result);
    }
    result.date = integer(data, "date");
    result.out = boolean(data, "out");
    result.self_id = context.self_peer_id;
    result.peer_id = parse_peer_id(&data["peer_id"]);
    result.from_id = if data["from_id"].is_null() {
        result.peer_id
    } else {
        parse_peer_id(&data["from_id"])
    };
    reply_header(&mut result, &data["reply_to"], true);
    if type_is(data, "MessageService") {
        result.action = parse_service_action(context, &data["action"], folder, result.date, dates)?;
        return Ok(result);
    }
    result.edited = integer(data, "edit_date");
    let forward = &data["fwd_from"];
    if type_is(forward, "MessageFwdHeader") {
        result.forwarded_from_id = parse_peer_id(&forward["from_id"]);
        result.forwarded_from_name = string(forward, "from_name").into();
        result.forwarded_date = integer(forward, "date");
        result.saved_from_chat_id = parse_peer_id(&forward["saved_from_peer"]);
        result.forwarded = result.forwarded_from_id != 0 || !result.forwarded_from_name.is_empty();
        result.show_forwarded_as_original = result.forwarded && result.saved_from_chat_id != 0;
    }
    result.signature = string(data, "post_author").into();
    result.via_bot_id = integer(data, "via_bot_id");
    if !data["media"].is_null() {
        context.bot_id = if result.via_bot_id != 0 {
            result.via_bot_id
        } else if result.forwarded_from_id != 0 && peer_is_user(result.forwarded_from_id) {
            peer_to_user(result.forwarded_from_id)
        } else {
            peer_to_user(result.from_id)
        };
        let media = parse_media(context, &data["media"], folder, result.date, dates);
        context.bot_id = 0;
        result.media = media?;
        if result.media.ttl != 0 && !result.out {
            result.media.reset_file();
            if let Some(MediaContent::Document(d)) = &mut result.media.content {
                d.thumb.file = File::default();
            }
        }
    }
    if type_is(&data["reply_markup"], "ReplyInlineMarkup") {
        result.inline_button_rows = button_rows_from_tl(&data["reply_markup"]);
    }
    result.text = parse_text(string(data, "message"), array(data, "entities"));
    if !data["reactions"].is_null() {
        result.reactions = parse_reactions(&data["reactions"]);
    }
    if !data["rich_message"].is_null() {
        result.rich_message = Some(parse_rich_message(
            context,
            &data["rich_message"],
            folder,
            result.date,
            dates,
        )?);
    }
    Ok(result)
}
pub fn parse_messages_list(
    self_id: i64,
    data: &[Value],
    folder: &str,
    dates: &DateContext,
) -> Result<BTreeMap<MessageId, Message>> {
    let mut context = ParseMediaContext {
        self_peer_id: self_id,
        ..Default::default()
    };
    let mut result = BTreeMap::new();
    for message in data {
        let parsed = parse_message(&mut context, message, folder, dates)?;
        result
            .entry(MessageId {
                channel_id: peer_to_channel(parsed.peer_id),
                msg_id: parsed.id,
            })
            .or_insert(parsed);
    }
    Ok(result)
}
pub fn parse_messages_slice(
    context: &mut ParseMediaContext,
    data: &[Value],
    users: &[Value],
    chats: &[Value],
    folder: &str,
    dates: &DateContext,
) -> Result<MessagesSlice> {
    let mut messages = Vec::new();
    for message in data.iter().rev() {
        messages.push(parse_message(context, message, folder, dates)?);
    }
    Ok(MessagesSlice {
        list: messages,
        peers: parse_peers_lists(users, chats),
    })
}
pub fn adjust_migrate_message_ids(slice: &mut MessagesSlice) {
    for message in &mut slice.list {
        message.id += MIGRATED_MESSAGES_ID_SHIFT;
        if message.reply_to_msg_id != 0 && message.reply_to_peer_id == 0 {
            message.reply_to_msg_id += MIGRATED_MESSAGES_ID_SHIFT;
        }
    }
}
pub fn single_message_date(data: &Value) -> i64 {
    if type_is(data, "messages.MessagesNotModified") || type_is(data, "MessagesNotModified") {
        return 0;
    }
    let Some(message) = array(data, "messages").first() else {
        return 0;
    };
    if type_is(message, "MessageEmpty") {
        0
    } else {
        integer(message, "date")
    }
}
pub fn single_message_before(data: &Value, date: i64) -> bool {
    let single = single_message_date(data);
    single > 0 && single < date
}
pub fn single_message_after(data: &Value, date: i64) -> bool {
    let single = single_message_date(data);
    single > 0 && single > date
}
pub fn skip_message_by_date(message: &Message, settings: &Settings) -> bool {
    !settings.includes_date(message.date)
}
