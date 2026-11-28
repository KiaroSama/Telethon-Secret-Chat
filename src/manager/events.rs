use crate::{enums, media::MediaReference};
#[derive(Clone)]
pub struct MessageReceived {
    pub chat_id: i32,
    pub random_id: i64,
    pub seq_no: i32,
    pub text: String,
    pub entities: Option<Vec<enums::MessageEntity>>,
    pub ttl: i32,
    pub media: Option<enums::DecryptedMessageMedia>,
    pub file: Option<grammers_tl_types::types::EncryptedFile>,
    pub reply_to: Option<i64>,
    pub media_reference: Option<MediaReference>,
}
impl std::fmt::Debug for MessageReceived {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageReceived")
            .field("chat_id", &self.chat_id)
            .field("seq_no", &self.seq_no)
            .field("chars", &self.text.chars().count())
            .field("media", &self.media.is_some())
            .finish()
    }
}
#[derive(Clone)]
pub enum Event {
    ChatRequested {
        chat_id: i32,
        peer_user_id: i64,
    },
    ChatReady {
        chat_id: i32,
        peer_user_id: i64,
        key_fingerprint: i64,
        key_hash: Option<Vec<u8>>,
    },
    ChatClosed {
        chat_id: i32,
        reason: &'static str,
        history_deleted: bool,
    },
    MessageReceived(Box<MessageReceived>),
    MessageAcknowledged {
        chat_id: i32,
        seq_no: i32,
        random_ids: Vec<i64>,
    },
    ServiceActionReceived {
        chat_id: i32,
        action_name: String,
        action: enums::DecryptedMessageAction,
        applied: bool,
    },
    DecryptFailed {
        chat_id: i32,
        reason: &'static str,
    },
    SendFailed {
        chat_id: i32,
        random_id: i64,
        cause: &'static str,
    },
}
impl Event {
    pub(crate) fn payload_size(&self) -> usize {
        use crate::Serializable;
        let dynamic = match self {
            Self::MessageReceived(m) => m
                .text
                .len()
                .saturating_add(
                    m.entities
                        .as_ref()
                        .map_or(0, |v| v.iter().map(|e| e.to_bytes().len()).sum()),
                )
                .saturating_add(m.media.as_ref().map_or(0, |m| m.to_bytes().len()))
                .saturating_add(
                    m.media_reference
                        .as_ref()
                        .map_or(0, |r| r.media_bytes().len()),
                ),
            Self::ServiceActionReceived {
                action,
                action_name,
                ..
            } => action.to_bytes().len().saturating_add(action_name.len()),
            Self::ChatReady { key_hash, .. } => key_hash.as_ref().map_or(0, Vec::len),
            Self::MessageAcknowledged { random_ids, .. } => {
                random_ids.len().saturating_mul(std::mem::size_of::<i64>())
            }
            _ => 0,
        };
        dynamic.saturating_add(512)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChatRequested { .. } => "ChatRequested",
            Self::ChatReady { .. } => "ChatReady",
            Self::ChatClosed { .. } => "ChatClosedEvent",
            Self::MessageReceived(_) => "MessageReceived",
            Self::MessageAcknowledged { .. } => "MessageAcknowledged",
            Self::ServiceActionReceived { .. } => "ServiceActionReceived",
            Self::DecryptFailed { .. } => "DecryptFailed",
            Self::SendFailed { .. } => "SendFailed",
        }
    }
    pub fn chat_id(&self) -> i32 {
        match self {
            Self::ChatRequested { chat_id, .. }
            | Self::ChatReady { chat_id, .. }
            | Self::ChatClosed { chat_id, .. }
            | Self::MessageAcknowledged { chat_id, .. }
            | Self::ServiceActionReceived { chat_id, .. }
            | Self::DecryptFailed { chat_id, .. }
            | Self::SendFailed { chat_id, .. } => *chat_id,
            Self::MessageReceived(message) => message.chat_id,
        }
    }
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(self.name())
            .field("chat_id", &self.chat_id())
            .finish()
    }
}
pub fn message_event(
    chat_id: i32,
    wrapper: crate::types::DecryptedMessageLayer,
    file: Option<Vec<u8>>,
) -> crate::error::Result<Option<MessageReceived>> {
    use crate::enums::DecryptedMessage as M;
    use grammers_tl_types::Deserializable;
    let seq_no = wrapper.out_seq_no;
    let (random_id, text, entities, ttl, media, reply_to) = match wrapper.message {
        M::Message(m) => (
            m.random_id,
            m.message,
            m.entities,
            m.ttl,
            m.media,
            m.reply_to_random_id,
        ),
        M::Legacy36b091de(m) => (
            m.random_id,
            m.message,
            m.entities,
            m.ttl,
            m.media,
            m.reply_to_random_id,
        ),
        M::Legacy204d3878(m) => (m.random_id, m.message, None, m.ttl, Some(m.media), None),
        M::Legacy1f814f1f(m) => (m.random_id, m.message, None, 0, Some(m.media), None),
        _ => return Ok(None),
    };
    let file = match file {
        Some(bytes) => match grammers_tl_types::enums::EncryptedFile::from_bytes(&bytes)
            .map_err(|_| crate::error::Error::Media)?
        {
            grammers_tl_types::enums::EncryptedFile::File(file) => Some(file),
            _ => None,
        },
        None => None,
    };
    let media_reference = match (&media, &file) {
        (Some(media), Some(file)) if crate::media::file_parts(media).is_ok() => {
            Some(MediaReference::new(chat_id, file, media)?)
        }
        _ => None,
    };
    Ok(Some(MessageReceived {
        chat_id,
        random_id,
        seq_no,
        text,
        entities,
        ttl,
        media,
        file,
        reply_to,
        media_reference,
    }))
}
