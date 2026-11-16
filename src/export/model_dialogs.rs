//! Desktop dialog metadata and list finalization rules.
use super::{
    error::{ExportError, Result},
    model::*,
    model_peers::{Chat, Peer, PeerUser, parse_chat, parse_user, type_is},
    settings::Settings,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogType {
    #[default]
    Unknown,
    #[serde(rename = "Self")]
    Self_,
    Replies,
    VerifyCodes,
    Personal,
    Bot,
    PrivateGroup,
    PrivateSupergroup,
    PublicSupergroup,
    PrivateChannel,
    PublicChannel,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DialogInfo {
    pub r#type: DialogType,
    pub name: String,
    pub last_name: String,
    pub input: Value,
    pub top_message_id: i64,
    pub top_message_date: i64,
    pub peer_id: i64,
    pub color_index: usize,
    pub migrated_from_input: Value,
    pub migrated_to_channel_id: i64,
    pub monoforum_broadcast_input: Value,
    pub splits: Vec<i64>,
    pub only_my_messages: bool,
    pub is_left_channel: bool,
    pub is_monoforum: bool,
    pub relative_path: String,
    pub messages_count_per_split: Vec<i64>,
}
impl Default for DialogInfo {
    fn default() -> Self {
        Self {
            r#type: DialogType::Unknown,
            name: String::new(),
            last_name: String::new(),
            input: json!({"_":"InputPeerEmpty"}),
            top_message_id: 0,
            top_message_date: 0,
            peer_id: 0,
            color_index: 0,
            migrated_from_input: json!({"_":"InputPeerEmpty"}),
            migrated_to_channel_id: 0,
            monoforum_broadcast_input: json!({"_":"InputPeerEmpty"}),
            splits: vec![],
            only_my_messages: false,
            is_left_channel: false,
            is_monoforum: false,
            relative_path: String::new(),
            messages_count_per_split: vec![],
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DialogsInfo {
    pub chats: Vec<DialogInfo>,
    pub left: Vec<DialogInfo>,
}
impl DialogsInfo {
    pub fn item(&self, index: usize) -> Option<&DialogInfo> {
        self.chats.get(index).or_else(|| {
            index
                .checked_sub(self.chats.len())
                .and_then(|i| self.left.get(i))
        })
    }
}
pub fn dialog_type_from_chat(chat: &Chat) -> DialogType {
    use DialogType as D;
    if chat.is_monoforum && !chat.is_monoforum_admin {
        D::Personal
    } else if chat.is_monoforum_admin && chat.is_monoforum_of_public_broadcast {
        D::PublicSupergroup
    } else if chat.is_monoforum_admin {
        D::PrivateSupergroup
    } else if chat.username.is_empty() {
        if chat.is_broadcast {
            D::PrivateChannel
        } else if chat.is_supergroup {
            D::PrivateSupergroup
        } else {
            D::PrivateGroup
        }
    } else if chat.is_broadcast {
        D::PublicChannel
    } else {
        D::PublicSupergroup
    }
}
pub fn dialog_type_from_user(user: &PeerUser) -> DialogType {
    if user.is_self {
        DialogType::Self_
    } else if user.is_replies {
        DialogType::Replies
    } else if user.is_verify_codes {
        DialogType::VerifyCodes
    } else if user.is_bot {
        DialogType::Bot
    } else {
        DialogType::Personal
    }
}
pub fn dialog_info_from_user(user: &PeerUser) -> DialogInfo {
    DialogInfo {
        input: Peer::User(user.clone()).input(),
        name: user.info.first_name.clone(),
        last_name: user.info.last_name.clone(),
        peer_id: user.id(),
        r#type: dialog_type_from_user(user),
        ..Default::default()
    }
}
pub fn dialog_info_from_chat(chat: &Chat) -> DialogInfo {
    DialogInfo {
        input: chat.input.clone(),
        name: chat.title.clone(),
        peer_id: chat.id(),
        r#type: dialog_type_from_chat(chat),
        migrated_to_channel_id: chat.migrated_to_channel_id,
        is_monoforum: chat.is_monoforum,
        monoforum_broadcast_input: if chat.is_monoforum_admin {
            chat.monoforum_broadcast_input.clone()
        } else {
            json!({"_":"InputPeerEmpty"})
        },
        ..Default::default()
    }
}
pub fn parse_dialogs_info_users(single: &Value, users: &[Value]) -> Result<DialogsInfo> {
    let id = if type_is(single, "InputPeerUser") {
        peer_from_user(integer(single, "user_id"))
    } else if type_is(single, "InputPeerSelf") {
        0
    } else {
        return Err(ExportError::InvalidOptions);
    };
    let mut result = DialogsInfo::default();
    for user in users {
        let user_id = peer_from_user(integer(user, "id"));
        let is_self = type_is(user, "User") && boolean(user, "is_self");
        if user_id == id || id == 0 && is_self {
            result.chats.push(dialog_info_from_user(&parse_user(user)));
        }
    }
    Ok(result)
}
pub fn parse_dialogs_info_chats(single: &Value, chats: &[Value]) -> Result<DialogsInfo> {
    let id = if type_is(single, "InputPeerChat") {
        peer_from_chat(integer(single, "chat_id"))
    } else if type_is(single, "InputPeerChannel") {
        peer_from_channel(integer(single, "channel_id"))
    } else {
        return Err(ExportError::InvalidOptions);
    };
    let mut result = DialogsInfo::default();
    for chat in chats {
        let channel = [
            "Channel",
            "ChannelForbidden",
            "Community",
            "CommunityForbidden",
        ]
        .iter()
        .any(|c| type_is(chat, c));
        let peer_id = if channel {
            peer_from_channel(integer(chat, "id"))
        } else {
            peer_from_chat(integer(chat, "id"))
        };
        if peer_id == id {
            result.chats.push(dialog_info_from_chat(&parse_chat(chat)));
        }
    }
    Ok(result)
}
pub fn add_migrate_from_slice(
    to: &mut DialogInfo,
    source: &DialogInfo,
    split_index: i64,
    splits_count: i64,
) -> bool {
    if !(type_is(&to.migrated_from_input, "InputPeerEmpty")
        || type_is(&to.migrated_from_input, "InputPeerChat")
            && peer_from_chat(integer(&to.migrated_from_input, "chat_id")) == source.peer_id)
    {
        return false;
    }
    for (split, count) in source.splits.iter().zip(&source.messages_count_per_split) {
        to.splits.push(split - splits_count);
        to.messages_count_per_split.push(*count);
    }
    to.migrated_from_input = source.input.clone();
    to.splits.push(split_index - splits_count);
    to.messages_count_per_split.push(0);
    true
}
pub fn settings_from_dialogs_type(kind: DialogType) -> u32 {
    use DialogType as D;
    match kind {
        D::Self_ | D::Personal => Settings::PERSONAL_CHATS,
        D::Bot => Settings::BOT_CHATS,
        D::PrivateGroup | D::PrivateSupergroup => Settings::PRIVATE_GROUPS,
        D::PrivateChannel => Settings::PRIVATE_CHANNELS,
        D::PublicSupergroup => Settings::PUBLIC_GROUPS,
        D::PublicChannel => Settings::PUBLIC_CHANNELS,
        _ => 0,
    }
}
pub fn finalize_dialogs_info(info: &mut DialogsInfo, settings: &Settings) -> Result<()> {
    let full = info.chats.len() + info.left.len();
    let digits = (full as i64 - 1).to_string().len();
    let mut index = 0;
    for dialog in &mut info.chats {
        index += 1;
        dialog.relative_path = if settings.only_single_peer() {
            String::new()
        } else {
            format!("chats/chat_{index:0digits$}/")
        };
        let setting = settings_from_dialogs_type(dialog.r#type);
        if setting == 0 {
            return Err(ExportError::InvalidRecord);
        }
        dialog.only_my_messages =
            dialog.r#type != DialogType::Personal && (settings.full_chats & setting) != setting;
        if dialog.messages_count_per_split.len() == dialog.splits.len() {
            let mut pairs = dialog
                .splits
                .iter()
                .copied()
                .zip(dialog.messages_count_per_split.iter().copied())
                .collect::<Vec<_>>();
            pairs.sort_by_key(|(split, _)| *split);
            (dialog.splits, dialog.messages_count_per_split) = pairs.into_iter().unzip();
        } else if dialog.messages_count_per_split.is_empty() {
            dialog.splits.sort();
        } else {
            return Err(ExportError::InvalidRecord);
        }
    }
    for dialog in &mut info.left {
        index += 1;
        dialog.relative_path = format!("chats/chat_{index:0digits$}/");
        dialog.only_my_messages = true;
    }
    Ok(())
}
