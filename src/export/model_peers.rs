//! Desktop users, chats and ordered peer maps.
use super::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const REPLIES_USER_ID: i64 = 1271266957;
pub const VERIFY_CODES_USER_ID: i64 = 489000;
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContactInfo {
    pub user_id: i64,
    pub first_name: String,
    pub last_name: String,
    pub phone_number: String,
    pub date: i64,
    pub color_index: usize,
}
impl ContactInfo {
    pub fn name(&self) -> String {
        match (self.first_name.is_empty(), self.last_name.is_empty()) {
            (true, _) => self.last_name.clone(),
            (_, true) => self.first_name.clone(),
            _ => format!("{} {}", self.first_name, self.last_name),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeerUser {
    pub bare_id: i64,
    pub info: ContactInfo,
    pub username: String,
    pub color_index: usize,
    pub is_bot: bool,
    pub is_self: bool,
    pub is_replies: bool,
    pub is_verify_codes: bool,
    pub input: Value,
}
impl PeerUser {
    pub fn id(&self) -> i64 {
        peer_from_user(self.bare_id)
    }
    pub fn name(&self) -> String {
        self.info.name()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chat {
    pub bare_id: i64,
    pub migrated_to_channel_id: i64,
    pub title: String,
    pub username: String,
    pub color_index: usize,
    pub is_monoforum: bool,
    pub is_broadcast: bool,
    pub is_supergroup: bool,
    pub is_monoforum_admin: bool,
    pub has_monoforum_admin_rights: bool,
    pub is_monoforum_of_public_broadcast: bool,
    pub monoforum_link_id: i64,
    pub input: Value,
    pub monoforum_broadcast_input: Value,
}
impl Chat {
    pub fn id(&self) -> i64 {
        if self.is_broadcast || self.is_supergroup || kind(&self.input) == "InputPeerChannel" {
            peer_from_channel(self.bare_id)
        } else {
            peer_from_chat(self.bare_id)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Peer {
    User(PeerUser),
    Chat(Chat),
}
impl Default for Peer {
    fn default() -> Self {
        Self::User(PeerUser::default())
    }
}
impl Peer {
    pub fn id(&self) -> i64 {
        match self {
            Self::User(u) => peer_from_user(u.info.user_id),
            Self::Chat(c) => c.id(),
        }
    }
    pub fn name(&self) -> String {
        match self {
            Self::User(u) => u.name(),
            Self::Chat(c) => c.title.clone(),
        }
    }
    pub fn color_index(&self) -> usize {
        match self {
            Self::User(u) => u.color_index,
            Self::Chat(c) => c.color_index,
        }
    }
    pub fn user(&self) -> Option<&PeerUser> {
        if let Self::User(u) = self {
            Some(u)
        } else {
            None
        }
    }
    pub fn chat(&self) -> Option<&Chat> {
        if let Self::Chat(c) = self {
            Some(c)
        } else {
            None
        }
    }
    pub fn input(&self) -> Value {
        match self {
            Self::User(u) if kind(&u.input) == "InputUser" => {
                json!({"_":"InputPeerUser","user_id":u.input["user_id"],"access_hash":u.input["access_hash"]})
            }
            Self::User(_) => json!({"_":"InputPeerEmpty"}),
            Self::Chat(c) => c.input.clone(),
        }
    }
}
pub type Peers = BTreeMap<i64, Peer>;
pub fn type_is(v: &Value, expected: &str) -> bool {
    let name = kind(v);
    name == expected
        || name
            .get(..1)
            .is_some_and(|c| c.eq_ignore_ascii_case(&expected[..1]))
            && name.get(1..) == expected.get(1..)
}
fn custom_color(v: &Value, fallback: usize) -> usize {
    if type_is(v, "PeerColor") {
        v.get("color")
            .and_then(Value::as_u64)
            .map_or(fallback, |n| n as usize)
    } else {
        fallback
    }
}
pub fn parse_contact_info(data: &Value) -> ContactInfo {
    let mut r = ContactInfo::default();
    if type_is(data, "User") {
        r.user_id = integer(data, "id");
        r.color_index = custom_color(&data["color"], peer_color_index(r.user_id));
        r.first_name = string(data, "first_name").into();
        r.last_name = string(data, "last_name").into();
        r.phone_number = string(data, "phone").into();
    } else if type_is(data, "UserEmpty") {
        r.user_id = integer(data, "id");
        r.color_index = peer_color_index(r.user_id);
    }
    r
}
pub fn parse_user(data: &Value) -> PeerUser {
    let mut r = PeerUser {
        info: parse_contact_info(data),
        input: json!({"_":"InputUserEmpty"}),
        ..Default::default()
    };
    if type_is(data, "User") {
        r.bare_id = integer(data, "id");
        r.color_index = custom_color(&data["color"], peer_color_index(r.bare_id));
        r.username = string(data, "username").into();
        r.is_bot = data.get("bot_info_version").is_some_and(|v| !v.is_null());
        r.is_self = boolean(data, "is_self");
        r.is_replies = !r.is_self && r.bare_id == REPLIES_USER_ID;
        r.is_verify_codes = !r.is_self && r.bare_id == VERIFY_CODES_USER_ID;
        r.input =
            json!({"_":"InputUser","user_id":r.bare_id,"access_hash":integer(data,"access_hash")});
    } else if type_is(data, "UserEmpty") {
        r.bare_id = integer(data, "id");
        r.input = json!({"_":"InputUser","user_id":integer(data,"id"),"access_hash":0});
    }
    r
}
pub fn parse_users_list(values: &[Value]) -> BTreeMap<i64, PeerUser> {
    let mut map = BTreeMap::new();
    for value in values {
        let u = parse_user(value);
        map.entry(u.info.user_id).or_insert(u);
    }
    map
}
pub fn parse_chat(data: &Value) -> Chat {
    let mut r = Chat {
        input: json!({"_":"InputPeerEmpty"}),
        monoforum_broadcast_input: json!({"_":"InputPeerEmpty"}),
        ..Default::default()
    };
    let id = integer(data, "id");
    if ["Chat", "ChatEmpty", "ChatForbidden"]
        .iter()
        .any(|n| type_is(data, n))
    {
        r.bare_id = id;
        r.title = string(data, "title").into();
        r.input = json!({"_":"InputPeerChat","chat_id":id});
        if type_is(&data["migrated_to"], "InputChannel") {
            r.migrated_to_channel_id = integer(&data["migrated_to"], "channel_id");
        }
    } else if [
        "Channel",
        "ChannelForbidden",
        "Community",
        "CommunityForbidden",
    ]
    .iter()
    .any(|n| type_is(data, n))
    {
        r.bare_id = id;
        r.title = string(data, "title").into();
        r.input = json!({"_":"InputPeerChannel","channel_id":id,"access_hash":integer(data,"access_hash")});
        if type_is(data, "Channel") || type_is(data, "ChannelForbidden") {
            r.is_monoforum = boolean(data, "monoforum");
            r.is_broadcast = boolean(data, "broadcast");
            r.is_supergroup = boolean(data, "megagroup");
        }
        if type_is(data, "Channel") {
            r.color_index = custom_color(&data["color"], peer_color_index(id));
            r.has_monoforum_admin_rights = r.is_broadcast
                && (boolean(data, "creator")
                    || boolean(&data["admin_rights"], "manage_direct_messages"));
            r.monoforum_link_id = integer(data, "linked_monoforum_id");
            r.username = string(data, "username").into();
        }
    }
    r
}
pub fn parse_peers_lists(users: &[Value], chats: &[Value]) -> Peers {
    let mut map = Peers::new();
    for v in users {
        let p = Peer::User(parse_user(v));
        map.entry(p.id()).or_insert(p);
    }
    for v in chats {
        let p = Peer::Chat(parse_chat(v));
        map.entry(p.id()).or_insert(p);
    }
    let links = map
        .values()
        .filter_map(|p| p.chat())
        .map(|c| {
            (
                c.id(),
                (c.has_monoforum_admin_rights, !c.username.is_empty()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for p in map.values_mut() {
        if let Peer::Chat(c) = p
            && c.is_monoforum
            && let Some((admin, public)) = links.get(&peer_from_channel(c.monoforum_link_id))
        {
            c.is_monoforum_admin = *admin;
            c.is_monoforum_of_public_broadcast = *public;
        }
    }
    map
}
pub fn empty_peer(id: i64) -> Peer {
    if peer_is_user(id) {
        Peer::User(parse_user(&json!({"_":"UserEmpty","id":peer_to_user(id)})))
    } else if peer_is_channel(id) {
        Peer::Chat(parse_chat(
            &json!({"_":"ChannelForbidden","id":peer_to_channel(id)}),
        ))
    } else {
        Peer::Chat(parse_chat(&json!({"_":"ChatEmpty","id":peer_to_chat(id)})))
    }
}
impl From<&User> for PeerUser {
    fn from(user: &User) -> Self {
        let mut r = parse_user(
            &json!({"_":"User","id":user.id,"first_name":user.first_name,"last_name":user.last_name,"username":user.username,"phone":user.phone_number,"access_hash":user.access_hash,"is_self":user.is_self,"bot_info_version":if user.is_bot{Some(0)}else{None}}),
        );
        r.is_bot = user.is_bot;
        r
    }
}
