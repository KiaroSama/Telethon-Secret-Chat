//! The ordered value builder shared by message and service JSON serialization.
use super::{
    error::{ExportError, Result},
    format::DesktopNumber,
    json_serialize::*,
    model::*,
    model_message::Message,
    model_peers::{Peer, PeerUser, Peers},
};
pub struct MessageValues<'a> {
    pub context: &'a mut JsonContext,
    pub message: &'a Message,
    pub peers: &'a Peers,
    pub values: Pairs,
}
impl<'a> MessageValues<'a> {
    pub fn new(context: &'a mut JsonContext, message: &'a Message, peers: &'a Peers) -> Self {
        Self {
            context,
            message,
            peers,
            values: vec![],
        }
    }
    pub fn bare(&mut self, key: &str, value: Vec<u8>) {
        if !value.is_empty() {
            self.values.push((key.into(), value));
        }
    }
    pub fn number(&mut self, key: &str, value: impl DesktopNumber) {
        self.bare(key, number(value));
    }
    pub fn boolean(&mut self, key: &str, value: bool) {
        self.bare(
            key,
            if value {
                b"true".to_vec()
            } else {
                b"false".to_vec()
            },
        );
    }
    pub fn string(&mut self, key: &str, value: impl AsRef<[u8]>) {
        let bytes = value.as_ref();
        if !bytes.is_empty() {
            self.bare(key, serialize_string(bytes));
        }
    }
    pub fn peer(&mut self, key: &str, id: i64) {
        self.bare(key, Self::wrap_peer_id(id));
    }
    pub fn wrap_peer_id(id: i64) -> Vec<u8> {
        serialize_string(if peer_to_chat(id) != 0 {
            format!("chat{}", peer_to_chat(id))
        } else if peer_to_channel(id) != 0 {
            format!("channel{}", peer_to_channel(id))
        } else {
            format!("user{}", peer_to_user(id))
        })
    }
    pub fn peer_of(&self, id: i64) -> Peer {
        self.peers.get(&id).cloned().unwrap_or_default()
    }
    pub fn user_of(&self, id: i64) -> PeerUser {
        self.peer_of(peer_from_user(id))
            .user()
            .cloned()
            .unwrap_or_default()
    }
    pub fn wrap_peer_name(&self, id: i64) -> Vec<u8> {
        string_allow_null(&self.peer_of(id).name())
    }
    pub fn wrap_user_name(&self, id: i64) -> Vec<u8> {
        string_allow_null(&self.user_of(id).name())
    }
    pub fn push_from(&mut self, label: &str) {
        if self.message.from_id != 0 {
            self.bare(label, self.wrap_peer_name(self.message.from_id));
            self.peer(&format!("{label}_id"), self.message.from_id);
        }
    }
    pub fn push_reply_to_msg_id(&mut self, label: &str) {
        if self.message.reply_to_msg_id != 0 {
            self.number(label, self.message.reply_to_msg_id);
            if self.message.reply_to_peer_id != 0 {
                self.peer("reply_to_peer_id", self.message.reply_to_peer_id);
            }
        }
    }
    pub fn push_user_names(&mut self, ids: &[i64], label: &str) {
        let names = ids
            .iter()
            .map(|id| self.wrap_user_name(*id))
            .collect::<Vec<_>>();
        self.bare(label, serialize_array(self.context, &names));
    }
    pub fn push_actor(&mut self) {
        self.push_from("actor");
    }
    pub fn push_action(&mut self, name: &str) {
        self.string("action", name);
    }
    pub fn push_ttl(&mut self, label: &str) {
        if self.message.media.ttl != 0 {
            self.number(label, self.message.media.ttl);
        }
    }
    pub fn push_path(&mut self, file: &File, label: &str) -> Result<()> {
        match file.skip_reason {
            SkipReason::None => self.string(label, format_file_path(file)),
            SkipReason::Unavailable => {
                self.string(label, "(File unavailable, please try again later)")
            }
            SkipReason::FileSize => self.string(
                label,
                "(File exceeds maximum size. Change data exporting settings to download.)",
            ),
            SkipReason::FileType => self.string(
                label,
                "(File not included. Change data exporting settings to download.)",
            ),
            SkipReason::DateLimits => return Err(ExportError::InvalidRecord),
        }
        Ok(())
    }
    pub fn push_photo(&mut self, image: &Image) -> Result<()> {
        self.push_path(&image.file, "photo")?;
        self.number("photo_file_size", image.file.size);
        if image.width != 0 && image.height != 0 {
            self.number("width", image.width);
            self.number("height", image.height);
        }
        Ok(())
    }
    pub fn push_spoiler(&mut self, spoilered: bool) {
        if spoilered {
            self.boolean("media_spoiler", true);
        }
    }
}
