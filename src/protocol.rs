//! Synchronous durable protocol transitions. An application executes returned
//! effects only after the corresponding snapshot is committed successfully.
use crate::{
    Serializable,
    chat::{Chat, ChatSnapshot, ChatState, DhRecord, Envelope, Outgoing},
    crypto::{SharedKey, decrypt_frame, encrypt_frame, random_id},
    enums,
    error::{Error, Result},
    framing, sequence,
    storage::{State, Storage},
    types,
};

/// A network effect carries protected bytes and intentionally has no Debug.
#[derive(Clone)]
pub struct SendEffect {
    pub chat_id: i32,
    pub access_hash: i64,
    pub outgoing: Outgoing,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    MessageAvailable {
        chat_id: i32,
        sequence: i32,
        random_id: i64,
    },
    MessageAcknowledged {
        chat_id: i32,
        random_ids: Vec<i64>,
    },
    ChatClosed {
        chat_id: i32,
    },
    LayerChanged {
        chat_id: i32,
        layer: i32,
    },
    DecryptFailed {
        chat_id: i32,
    },
}
#[derive(Default)]
pub struct Effects {
    pub sends: Vec<SendEffect>,
    pub events: Vec<Event>,
}

pub struct Protocol<S: Storage> {
    storage: S,
    state: State,
    running: bool,
}
impl<S: Storage> Protocol<S> {
    pub fn open(storage: S, authorization: String) -> Result<Self> {
        let state = match storage.load()? {
            Some(state) => {
                if state.authorization != authorization {
                    return Err(Error::Authorization);
                }
                state
            }
            None => State::empty(authorization)?,
        };
        state.validate()?;
        Ok(Self {
            storage,
            state,
            running: false,
        })
    }
    pub fn start(&mut self) -> Result<Vec<SendEffect>> {
        if self.running {
            return Err(Error::ChatState);
        }
        self.state.validate()?;
        self.running = true;
        Ok(self
            .state
            .chats
            .values()
            .filter(|c| !c.paused)
            .flat_map(|chat| {
                chat.outbox
                    .values()
                    .filter(|o| o.pending)
                    .cloned()
                    .map(|outgoing| SendEffect {
                        chat_id: chat.id,
                        access_hash: chat.access_hash,
                        outgoing,
                    })
            })
            .collect())
    }
    pub fn stop(&mut self) -> Result<()> {
        self.storage.commit(&self.state)?;
        self.running = false;
        Ok(())
    }
    pub fn snapshot(&self, id: i32) -> Result<ChatSnapshot> {
        Ok(self
            .state
            .chats
            .get(&id)
            .ok_or(Error::ChatState)?
            .snapshot())
    }
    pub fn snapshots(&self) -> Vec<ChatSnapshot> {
        self.state.chats.values().map(Chat::snapshot).collect()
    }
    fn transaction<T>(&mut self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        if !self.running {
            return Err(Error::NotRunning);
        }
        let mut next = self.state.clone();
        let result = f(&mut next)?;
        next.validate()?;
        self.storage.commit(&next)?;
        self.state = next;
        Ok(result)
    }
    /// Install an already-validated handshake. The transport driver must compare
    /// the server's claimed fingerprint before calling this method.
    pub fn establish(
        &mut self,
        mut chat: Chat,
        key: SharedKey,
        dh: DhRecord,
        now: u64,
    ) -> Result<SendEffect> {
        let config = crate::dh::DhConfig::new(dh.g, &dh.prime)?;
        if config.generator() != dh.g {
            return Err(Error::DhParameters);
        }
        self.transaction(|state| {
            if state.chats.contains_key(&chat.id) {
                return Err(Error::ChatState);
            }
            chat.dh = Some(dh);
            chat.adopt_key(key);
            chat.state = ChatState::Ready;
            chat.rekeyed_at = now;
            let effect = prepare_action(
                &mut chat,
                types::DecryptedMessageActionNotifyLayer { layer: 144 }.into(),
            )?;
            state.chats.insert(chat.id, chat);
            Ok(effect)
        })
    }
    pub fn send_message(
        &mut self,
        id: i32,
        text: String,
        entities: Option<Vec<enums::MessageEntity>>,
        reply_to: Option<i64>,
    ) -> Result<SendEffect> {
        if text.len() > 1024 * 1024 {
            return Err(Error::Capacity);
        }
        self.transaction(|state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            let message = types::DecryptedMessage {
                no_webpage: false,
                silent: false,
                random_id: random_id()?,
                ttl: chat.ttl,
                message: text,
                media: None,
                entities,
                via_bot_name: None,
                reply_to_random_id: reply_to,
                grouped_id: None,
            };
            prepare(chat, message.into(), false, None)
        })
    }
    pub fn send_action(
        &mut self,
        id: i32,
        action: enums::DecryptedMessageAction,
    ) -> Result<SendEffect> {
        self.transaction(|state| {
            prepare_action(state.chats.get_mut(&id).ok_or(Error::ChatState)?, action)
        })
    }
    pub fn rekey(&mut self, id: i32) -> Result<Option<SendEffect>> {
        self.transaction(|state| {
            crate::rekey::start(state.chats.get_mut(&id).ok_or(Error::ChatState)?)
        })
    }
    pub fn confirm_sent(&mut self, id: i32, seq_no: i32, random_id: i64) -> Result<()> {
        self.transaction(|state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            // A peer acknowledgement can remove the retained row before its RPC
            // response arrives; that response then has nothing left to mark.
            if let Some(item) = chat.outbox.get_mut(&seq_no) {
                if item.random_id != random_id {
                    return Err(Error::Sequence);
                }
                item.pending = false;
            }
            Ok(())
        })
    }
    pub fn retry_pending(&self, id: i32) -> Result<Vec<SendEffect>> {
        if !self.running {
            return Err(Error::NotRunning);
        }
        let chat = self.state.chats.get(&id).ok_or(Error::ChatState)?;
        chat.require_ready()?;
        Ok(chat
            .outbox
            .values()
            .filter(|m| m.pending)
            .cloned()
            .map(|outgoing| SendEffect {
                chat_id: id,
                access_hash: chat.access_hash,
                outgoing,
            })
            .collect())
    }
    pub fn receive(
        &mut self,
        id: i32,
        frame: &[u8],
        file: Option<Vec<u8>>,
        date: i32,
    ) -> Result<Effects> {
        let chat = self.state.chats.get(&id).ok_or(Error::ChatState)?;
        chat.require_ready()?;
        let fingerprint = i64::from_le_bytes(
            frame
                .get(..8)
                .ok_or(Error::FrameRejected)?
                .try_into()
                .map_err(|_| Error::FrameRejected)?,
        );
        let pending = match &chat.exchange {
            Some(crate::chat::Exchange::Accepted { key, .. }) => Some(key),
            _ => None,
        };
        let key = [chat.key.as_ref(), pending, chat.previous_key.as_ref()]
            .into_iter()
            .flatten()
            .find(|key| key.fingerprint() == fingerprint)
            .ok_or(Error::FrameRejected)?;
        let body = decrypt_frame(key, frame, chat.in_direction())?;
        let wrapper = framing::unwrap(&body)?;
        if !sequence::preflight(chat, &wrapper)? {
            return Ok(Effects::default());
        }
        self.transaction(|state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            let accepted = sequence::accept(
                chat,
                Envelope {
                    body: body.to_vec(),
                    file,
                    date,
                },
            )?;
            let mut effects = Effects::default();
            if !accepted.acknowledged.is_empty() {
                effects.events.push(Event::MessageAcknowledged {
                    chat_id: id,
                    random_ids: accepted.acknowledged,
                });
            }
            for (seq, envelope) in accepted.ready {
                let wrapper = framing::unwrap(&envelope.body)?;
                let random_id = wrapper.message.random_id();
                match &wrapper.message {
                    enums::DecryptedMessage::Service(service) => {
                        apply_action(chat, &service.action, &mut effects)?
                    }
                    enums::DecryptedMessage::ServiceLegacyaa48327d(service) => {
                        apply_action(chat, &service.action, &mut effects)?
                    }
                    _ => {
                        chat.deliveries.insert(seq, envelope);
                        effects.events.push(Event::MessageAvailable {
                            chat_id: id,
                            sequence: seq,
                            random_id,
                        });
                    }
                }
            }
            if let Some((start, end)) = accepted.resend {
                effects.sends.push(prepare_action(
                    chat,
                    types::DecryptedMessageActionResend {
                        start_seq_no: start,
                        end_seq_no: end,
                    }
                    .into(),
                )?);
                chat.resend_due = None;
            }
            crate::rekey::observe_key(chat, fingerprint, u64::try_from(date).unwrap_or(0));
            crate::rekey::retire_if_settled(chat);
            Ok(effects)
        })
    }
    pub fn delivery(&self, id: i32, sequence: i32) -> Result<Option<Envelope>> {
        Ok(self
            .state
            .chats
            .get(&id)
            .ok_or(Error::ChatState)?
            .deliveries
            .get(&sequence)
            .cloned())
    }
    pub fn pending_deliveries(&self, id: i32) -> Result<Vec<i32>> {
        Ok(self
            .state
            .chats
            .get(&id)
            .ok_or(Error::ChatState)?
            .deliveries
            .keys()
            .copied()
            .collect())
    }
    pub fn acknowledge_delivery(&mut self, id: i32, sequence: i32) -> Result<()> {
        self.transaction(|state| {
            state
                .chats
                .get_mut(&id)
                .ok_or(Error::ChatState)?
                .deliveries
                .remove(&sequence);
            Ok(())
        })
    }
    pub fn pause(&mut self, id: i32) -> Result<()> {
        self.transaction(|state| {
            state.chats.get_mut(&id).ok_or(Error::ChatState)?.paused = true;
            Ok(())
        })
    }
    pub fn resume(&mut self, id: i32) -> Result<()> {
        self.transaction(|state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            chat.validate()?;
            if matches!(chat.state, ChatState::Suspended | ChatState::Closed) {
                return Err(Error::ChatState);
            }
            chat.paused = false;
            Ok(())
        })
    }
    pub fn close(&mut self, id: i32) -> Result<()> {
        self.transaction(|state| {
            state.chats.get_mut(&id).ok_or(Error::ChatState)?.close();
            Ok(())
        })
    }
}
fn prepare(
    chat: &mut Chat,
    message: enums::DecryptedMessage,
    service: bool,
    file: Option<Vec<u8>>,
) -> Result<SendEffect> {
    chat.require_ready()?;
    let random_id = message.random_id();
    let sequence = framing::transform_out_seq_no(chat.out_seq_no, chat.is_outbound)?;
    let wrapper = framing::wrap(
        message,
        framing::outgoing_layer(chat.capability_layer),
        framing::transform_in_seq_no(chat.in_seq_no, chat.is_outbound)?,
        sequence,
    )?;
    let frame = encrypt_frame(
        chat.key.as_ref().ok_or(Error::ChatState)?,
        &wrapper.to_bytes(),
        chat.out_direction(),
    )?;
    let outgoing = Outgoing {
        seq_no: sequence,
        random_id,
        frame,
        service,
        file,
        pending: true,
    };
    chat.out_seq_no = chat
        .out_seq_no
        .checked_add(1)
        .filter(|n| *n <= i32::MAX / 2)
        .ok_or(Error::Sequence)?;
    chat.messages_since_rekey = chat.messages_since_rekey.saturating_add(1);
    chat.outbox.insert(sequence, outgoing.clone());
    Ok(SendEffect {
        chat_id: chat.id,
        access_hash: chat.access_hash,
        outgoing,
    })
}
pub(crate) fn prepare_action(
    chat: &mut Chat,
    action: enums::DecryptedMessageAction,
) -> Result<SendEffect> {
    if let enums::DecryptedMessageAction::SetMessageTtl(ttl) = &action {
        if ttl.ttl_seconds < 0 {
            return Err(Error::Media);
        }
        chat.ttl = ttl.ttl_seconds;
    }
    if matches!(action, enums::DecryptedMessageAction::FlushHistory) {
        chat.deliveries.clear();
    }
    let message = types::DecryptedMessageService {
        random_id: random_id()?,
        action,
    };
    prepare(chat, message.into(), true, None)
}
fn apply_action(
    chat: &mut Chat,
    action: &enums::DecryptedMessageAction,
    effects: &mut Effects,
) -> Result<()> {
    if matches!(
        action,
        enums::DecryptedMessageAction::RequestKey(_)
            | enums::DecryptedMessageAction::AcceptKey(_)
            | enums::DecryptedMessageAction::CommitKey(_)
            | enums::DecryptedMessageAction::AbortKey(_)
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::ChatState)?
            .as_secs();
        crate::rekey::handle(chat, action, effects, now)?;
        return Ok(());
    }
    match action {
        enums::DecryptedMessageAction::NotifyLayer(layer) => {
            if layer.layer < 0 {
                return Err(Error::WrapperRejected);
            }
            chat.capability_layer = chat.capability_layer.max(layer.layer);
            effects.events.push(Event::LayerChanged {
                chat_id: chat.id,
                layer: chat.capability_layer,
            });
        }
        enums::DecryptedMessageAction::SetMessageTtl(ttl) => chat.ttl = ttl.ttl_seconds.max(0),
        enums::DecryptedMessageAction::Resend(span) => {
            for outgoing in sequence::answer_resend(chat, span.start_seq_no, span.end_seq_no)? {
                effects.sends.push(SendEffect {
                    chat_id: chat.id,
                    access_hash: chat.access_hash,
                    outgoing,
                });
            }
        }
        enums::DecryptedMessageAction::FlushHistory => chat.deliveries.clear(),
        enums::DecryptedMessageAction::DeleteMessages(delete) => {
            let mut removed = Vec::new();
            for (seq, envelope) in &chat.deliveries {
                if delete
                    .random_ids
                    .contains(&framing::unwrap(&envelope.body)?.message.random_id())
                {
                    removed.push(*seq);
                }
            }
            for seq in removed {
                chat.deliveries.remove(&seq);
            }
        }
        _ => {}
    }
    Ok(())
}
