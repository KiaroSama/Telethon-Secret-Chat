//! Synchronous durable protocol transitions. An application executes returned
//! effects only after the corresponding snapshot is committed successfully.
#[path = "access.rs"]
mod access;
#[path = "actions.rs"]
mod actions;
#[path = "custody.rs"]
mod custody;
#[path = "effects.rs"]
mod effects;
#[path = "withdrawal.rs"]
mod withdrawal;

use crate::{
    Serializable,
    chat::{Chat, ChatSnapshot, ChatState, DhRecord, Envelope},
    crypto::{SharedKey, decrypt_frame, random_id},
    enums,
    error::{Error, Result},
    framing, sequence,
    storage::{State, Storage},
    types,
};
pub(crate) use actions::prepare_action;
use actions::{apply_action, prepare};
pub use effects::*;
pub(crate) use withdrawal::rewrite_retained_as_deletes;

pub struct Protocol<S: Storage> {
    storage: S,
    state: State,
    running: bool,
}
impl<S: Storage> Protocol<S> {
    pub fn open(mut storage: S, authorization: String) -> Result<Self> {
        let state = match storage.load()? {
            Some(state) => {
                if state.authorization != authorization {
                    return Err(Error::Authorization);
                }
                state
            }
            None => State::empty(authorization)?,
        };
        state.validate_restored()?;
        if storage.repair_pending() {
            storage.commit(&state)?;
        }
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
        if let Some(state) = self.storage.resume_storage()? {
            if state.authorization != self.state.authorization {
                return Err(Error::Authorization);
            }
            state.validate()?;
            self.state = state;
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
        if !self.running {
            return Ok(());
        }
        if self.state.mode == crate::storage::StoreMode::Protected {
            let mut state = self.state.clone();
            for chat in state.chats.values_mut() {
                chat.state = ChatState::Suspended;
                chat.paused = true;
                chat.outbox.clear();
                chat.gaps.clear();
                chat.deliveries.clear();
                chat.service_deliveries.clear();
            }
            crate::custody::transient::suspend_requests(&mut state);
            if let Err(error) = self.storage.commit(&state) {
                if let Some(view) = self.storage.committed_view() {
                    self.state = view;
                }
                return Err(error);
            }
            self.state = self.storage.committed_view().unwrap_or(state);
        } else {
            if let Err(error) = self.storage.commit(&self.state) {
                if let Some(view) = self.storage.committed_view() {
                    self.state = view;
                }
                return Err(error);
            }
        }
        self.storage.suspend_storage()?;
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
    pub(crate) fn transaction<T>(&mut self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        if !self.running {
            return Err(Error::NotRunning);
        }
        let mut next = self.state.clone();
        let result = f(&mut next)?;
        next.validate()?;
        if let Err(error) = self.storage.commit(&next) {
            if let Some(view) = self.storage.committed_view() {
                self.state = view;
            }
            return Err(error);
        }
        self.state = self.storage.committed_view().unwrap_or(next);
        Ok(result)
    }
    fn ingress_transaction<T>(
        &mut self,
        id: i32,
        kind: crate::ingress::application::Kind,
        f: impl FnOnce(&mut State) -> Result<T>,
    ) -> Result<T> {
        crate::ingress::application::transition(kind, id, || self.transaction(f))
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
            chat.adopt_key(key)?;
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
        self.send_message_with_options(id, text, entities, reply_to, None, false)
    }

    pub(crate) fn send_message_with_options(
        &mut self,
        id: i32,
        text: String,
        entities: Option<Vec<enums::MessageEntity>>,
        reply_to: Option<i64>,
        ttl: Option<i32>,
        silent: bool,
    ) -> Result<SendEffect> {
        if ttl.is_some_and(|value| value < 0) {
            return Err(Error::MediaField { field: "ttl" });
        }
        self.admit(Some(id), crate::storage::Operation::Send, text.len())?;
        if text.len() > 1024 * 1024 {
            return Err(Error::Capacity);
        }
        self.transaction(|state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            let message = types::DecryptedMessage {
                no_webpage: false,
                silent,
                random_id: random_id()?,
                ttl: ttl.unwrap_or(chat.ttl),
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
        self.admit(
            Some(id),
            crate::storage::Operation::Send,
            action.to_bytes().len(),
        )?;
        self.transaction(|state| {
            prepare_action(state.chats.get_mut(&id).ok_or(Error::ChatState)?, action)
        })
    }
    pub fn rekey(&mut self, id: i32) -> Result<Option<SendEffect>> {
        self.admit(Some(id), crate::storage::Operation::Rekey, 0)?;
        self.transaction(|state| {
            crate::rekey::start(state.chats.get_mut(&id).ok_or(Error::ChatState)?)
        })
    }
    /// Confirm only the exact effect that completed. A late response cannot
    /// confirm replacement ciphertext produced by a concurrent withdrawal.
    pub fn confirm_sent(&mut self, effect: &SendEffect) -> Result<()> {
        self.confirm_sent_with_file(effect, None)
    }
    pub(crate) fn confirm_sent_with_file(
        &mut self,
        effect: &SendEffect,
        returned_file: Option<Vec<u8>>,
    ) -> Result<()> {
        self.transaction(|state| {
            let Some(chat) = state.chats.get_mut(&effect.chat_id) else {
                return Ok(());
            };
            // A peer acknowledgement can remove the retained row before its RPC
            // response arrives; that response then has nothing left to mark.
            if let Some(item) = chat.outbox.get_mut(&effect.outgoing.seq_no)
                && chat.access_hash == effect.access_hash
                && item.random_id == effect.outgoing.random_id
                && item.frame == effect.outgoing.frame
                && item.service == effect.outgoing.service
                && item.file == effect.outgoing.file
                && item.obligation_generation == effect.outgoing.obligation_generation
            {
                item.pending = false;
                if let Some(file) = returned_file {
                    item.file = Some(file);
                }
            }
            Ok(())
        })
    }
    pub fn retry_pending(&mut self, id: i32) -> Result<Vec<SendEffect>> {
        self.admit(Some(id), crate::storage::Operation::Retry, 0)?;
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
        // Telegram's ordinary encrypted envelope represents no attachment as an
        // EncryptedFile::Empty object. Normalize it before mailbox validation.
        let file = file
            .filter(|value| value != &grammers_tl_types::enums::EncryptedFile::Empty.to_bytes());
        self.admit(Some(id), crate::storage::Operation::Receive, frame.len())?;
        let chat = self.state.chats.get(&id).ok_or(Error::ChatState)?;
        chat.require_ready()?;
        let fingerprint = i64::from_le_bytes(
            frame
                .get(..8)
                .ok_or(Error::FrameRejected)?
                .try_into()
                .map_err(|_| Error::FrameRejected)?,
        );
        let key = crate::rekey::select_key(chat, fingerprint).ok_or(Error::FrameRejected)?;
        let body = decrypt_frame(key, frame, chat.in_direction())?;
        let header = framing::inspect_header(&body)?;
        if !sequence::preflight_header(chat, &header)? {
            return self.ingress_transaction(
                id,
                crate::ingress::application::Kind::Message,
                |_| Ok(Effects::default()),
            );
        }
        let wrapper = framing::unwrap(&body)?;
        self.ingress_transaction(id, crate::ingress::application::Kind::Message, |state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            let mut wrapper = wrapper;
            let mut answers = Vec::new();
            let action = match &mut wrapper.message {
                enums::DecryptedMessage::Service(service) => Some(&mut service.action),
                enums::DecryptedMessage::ServiceLegacyaa48327d(service) => {
                    Some(&mut service.action)
                }
                _ => None,
            };
            // Resend is an authenticated immediate protocol action, even behind
            // its own sequence gap. Rewrite only the retained decrypted action
            // so replay/drain cannot apply the same request twice.
            if let Some(action) = action
                && let enums::DecryptedMessageAction::Resend(span) = action
            {
                answers = sequence::answer_resend(chat, span.start_seq_no, span.end_seq_no)?;
                *action = enums::DecryptedMessageAction::Noop;
            }
            let accepted = sequence::accept(
                chat,
                Envelope {
                    body: enums::DecryptedMessageLayer::from(wrapper).to_bytes(),
                    file,
                    date,
                },
            )?;
            let mut effects = Effects::default();
            for (seq, envelope) in accepted.ready {
                let wrapper = framing::unwrap(&envelope.body)?;
                let random_id = wrapper.message.random_id();
                let action = match &wrapper.message {
                    enums::DecryptedMessage::Service(service) => Some(&service.action),
                    enums::DecryptedMessage::ServiceLegacyaa48327d(service) => {
                        Some(&service.action)
                    }
                    _ => None,
                };
                if let Some(action) = action {
                    // Service application is a durable mailbox step performed
                    // in the same order as ordinary callback/autosave delivery.
                    // Applying a later Delete/Flush here could erase an earlier
                    // ordinary message before it ever reached the application.
                    let _ = action;
                    chat.service_deliveries.insert(
                        seq,
                        crate::chat::ServiceDelivery {
                            envelope: envelope.clone(),
                            applied: None,
                        },
                    );
                } else {
                    chat.deliveries.insert(seq, envelope);
                    effects.events.push(Event::MessageAvailable {
                        chat_id: id,
                        sequence: seq,
                        random_id,
                    });
                }
            }
            // Later ready acknowledgements cannot remove ciphertext needed by
            // earlier ready actions. Consume acknowledgements only after actions.
            let acknowledged = chat
                .outbox
                .keys()
                .copied()
                .filter(|seq| seq / 2 < chat.peer_in_seq_no)
                .collect::<Vec<_>>();
            let mut random_ids = Vec::new();
            for seq in acknowledged {
                if let Some(row) = chat.outbox.remove(&seq) {
                    random_ids.push(row.random_id);
                }
            }
            if !random_ids.is_empty() {
                effects.events.push(Event::MessageAcknowledged {
                    chat_id: id,
                    random_ids,
                });
            }
            // Retrying must survive a crash after consuming the Resend request,
            // including when its requested row had already been RPC-confirmed.
            for mut outgoing in answers {
                outgoing.pending = true;
                outgoing.obligation_generation = outgoing
                    .obligation_generation
                    .checked_add(1)
                    .ok_or(Error::Capacity)?;
                chat.outbox.insert(outgoing.seq_no, outgoing.clone());
                effects.sends.push(SendEffect {
                    chat_id: id,
                    access_hash: chat.access_hash,
                    outgoing,
                });
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
            chat.messages_since_rekey = chat.messages_since_rekey.saturating_add(1);
            let adopted_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::ChatState)?
                .as_secs();
            if crate::rekey::observe_key(chat, fingerprint, adopted_at)? {
                effects
                    .sends
                    .push(prepare_action(chat, enums::DecryptedMessageAction::Noop)?);
            }
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
        self.close_with_reason(id, crate::error::CloseReason::Application)
    }
    pub fn close_with_reason(&mut self, id: i32, reason: crate::error::CloseReason) -> Result<()> {
        if !self.running {
            return Err(Error::NotRunning);
        }
        if matches!(
            self.mode(),
            crate::storage::StoreMode::Bounded | crate::storage::StoreMode::Ingress
        ) {
            crate::ingress::application::transition(
                crate::ingress::application::Kind::Terminal,
                id,
                || self.storage.mark_nonresumable(id),
            )?;
            if let Some(view) = self.storage.committed_view() {
                self.state = view;
            }
        }
        self.ingress_transaction(id, crate::ingress::application::Kind::Terminal, |state| {
            let mode = state.mode;
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            match mode {
                crate::storage::StoreMode::Durable => chat.close_with_reason(reason),
                crate::storage::StoreMode::Protected => {
                    chat.state = ChatState::Suspended;
                    chat.paused = true;
                    chat.gaps.clear();
                    chat.outbox.clear();
                    chat.deliveries.clear();
                    chat.service_deliveries.clear();
                }
                crate::storage::StoreMode::Bounded | crate::storage::StoreMode::Ingress => {
                    chat.paused = true;
                }
            }
            Ok(())
        })
    }
}
