//! Protected disk projection plus explicitly bounded payload-bearing RAM state.
use super::{TransientLimits, identities, now};
use crate::{
    chat::ChatState,
    error::{Error, Result},
    storage::{FileStorage, Operation, State, Storage, StoreMode},
};
use std::{collections::BTreeMap, path::Path};
#[cfg(test)]
#[path = "../../tests/support_protocol/transient_review.rs"]
pub(crate) mod review;
const MAX_STORE: usize = 4 * 1024 * 1024;
const MAX_CHAT: usize = 64 * 1024;
pub struct ProtectedFileStorage {
    backend: FileStorage,
}
impl ProtectedFileStorage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let backend = FileStorage::open(path)?;
        if let Some(state) = backend.load()? {
            validate_protected(&state)?;
        }
        Ok(Self { backend })
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingCreate {
    peer: Vec<u8>,
    peer_user_id: i64,
    request_id: i32,
    secret: crate::dh::SecretExponent,
    dh: crate::chat::DhRecord,
}
fn validate_pending(bytes: &[u8]) -> Result<()> {
    use grammers_tl_types::Deserializable;
    let record: PendingCreate = serde_json::from_slice(bytes).map_err(|_| Error::InvalidState)?;
    if record.peer_user_id <= 0 || record.peer.len() > 1024 || record.secret.expose()[0] & 0x80 == 0
    {
        return Err(Error::InvalidState);
    }
    let _ = record.request_id;
    crate::dh::DhConfig::new(record.dh.g, &record.dh.prime)?;
    let mut cursor = grammers_tl_types::deserialize::Cursor::from_slice(&record.peer);
    grammers_tl_types::enums::InputUser::deserialize(&mut cursor)
        .map_err(|_| Error::InvalidState)?;
    if cursor.pos() != record.peer.len() {
        return Err(Error::InvalidState);
    }
    Ok(())
}
fn validate_protected(state: &State) -> Result<()> {
    state.validate()?;
    if state.mode != StoreMode::Protected
        || state.chats.len() > 64
        || !state.settings.is_empty()
        || serde_json::to_vec(state)
            .map_err(|_| Error::InvalidState)?
            .len()
            > MAX_STORE
    {
        return Err(Error::InvalidState);
    }
    for chat in state.chats.values() {
        if !chat.outbox.is_empty()
            || !chat.gaps.is_empty()
            || !chat.deliveries.is_empty()
            || !chat.service_deliveries.is_empty()
            || serde_json::to_vec(chat)
                .map_err(|_| Error::InvalidState)?
                .len()
                > MAX_CHAT
        {
            return Err(Error::InvalidState);
        }
    }
    for (name, value) in &state.private_records {
        if !(name.starts_with("pending-create/") || name.starts_with("suspended-create/"))
            || value.as_bytes().len() > MAX_CHAT
        {
            return Err(Error::InvalidState);
        }
        validate_pending(value.as_bytes())?;
    }
    Ok(())
}
impl Storage for ProtectedFileStorage {
    fn suspend_storage(&mut self) -> Result<()> {
        self.backend.suspend_storage()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        let state = self.backend.resume_storage()?;
        if let Some(state) = &state {
            validate_protected(state)?;
        }
        Ok(state)
    }
    fn load(&self) -> Result<Option<State>> {
        let state = self.backend.load()?;
        if let Some(state) = &state {
            validate_protected(state)?;
        }
        Ok(state)
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        validate_protected(state)?;
        if let Some(previous) = self.backend.load()?
            && (previous.authorization != state.authorization
                || previous.chats.iter().any(|(id, old)| {
                    state.chats.get(id).is_none_or(|new| {
                        new.state == ChatState::Closed
                            || old.key.is_some() && new.key.is_none()
                            || old.state == ChatState::Suspended
                                && new.state != ChatState::Suspended
                    })
                }))
        {
            return Err(Error::Paused);
        }
        self.backend.commit(state)
    }
}
/// No ordinary message, service frame or media reference is passed to `protected`.
/// Runtime queues remain in this wrapper and are permanently unusable after restart.
pub struct TransientStorage<S: Storage> {
    protected: S,
    state: State,
    limits: TransientLimits,
    created: BTreeMap<(i32, String), u64>,
    attempts: BTreeMap<(i32, i32), u8>,
    external: BTreeMap<u64, ExternalReservation>,
    admitted_chat: Option<i32>,
    refused: bool,
}
#[derive(Clone)]
struct ExternalReservation {
    chat: Option<i32>,
    bytes: usize,
    created_ms: u64,
    expired: bool,
}
impl<S: Storage> TransientStorage<S> {
    pub fn new(mut protected: S, authorization: String, limits: TransientLimits) -> Result<Self> {
        limits.validate()?;
        let mut state = match protected.load()? {
            Some(mut state) => {
                validate_protected(&state)?;
                if state.authorization != authorization {
                    return Err(Error::Authorization);
                }
                for chat in state.chats.values_mut() {
                    chat.state = ChatState::Suspended;
                    chat.paused = true;
                }
                suspend_requests(&mut state);
                state
            }
            None => {
                let mut state = State::empty(authorization)?;
                state.mode = StoreMode::Protected;
                state
            }
        };
        state.settings.clear();
        protected.commit(&state)?;
        Ok(Self {
            protected,
            state,
            limits,
            created: BTreeMap::new(),
            attempts: BTreeMap::new(),
            external: BTreeMap::new(),
            admitted_chat: None,
            refused: false,
        })
    }
    pub fn suspend(&mut self, id: i32) -> Result<()> {
        let mut state = self.state.clone();
        let chat = state
            .chats
            .get_mut(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?;
        scrub_chat(chat);
        self.persist_suspension(state)
    }
    // Suspension can only remove volatile custody. Publish its safe RAM state
    // even after a backend refusal, while retaining every key and counter.
    fn persist_suspension(&mut self, state: State) -> Result<()> {
        self.admitted_chat = None;
        let projected = projection(&state);
        validate_protected(&projected)?;
        let result = self.protected.commit(&projected);
        if result.is_err() {
            self.refused = true;
        }
        self.state = state;
        self.created.retain(|(id, identity), _| {
            self.state
                .chats
                .get(id)
                .is_some_and(|chat| identities(chat).contains(identity))
        });
        self.attempts.retain(|(id, seq), _| {
            self.state
                .chats
                .get(id)
                .is_some_and(|chat| chat.outbox.contains_key(seq))
        });
        result
    }
    pub fn limits(&self) -> &TransientLimits {
        &self.limits
    }
    fn expired(&self, id: i32, timestamp: u64) -> bool {
        self.created.iter().any(|((chat, _), time)| {
            *chat == id
                && timestamp.saturating_sub(*time) as f64 >= self.limits.retention_seconds * 1000.0
        })
    }
    fn check(&self, state: &State) -> Result<()> {
        self.limits.validate()?;
        let count = state
            .chats
            .values()
            .map(|c| {
                c.outbox.len() + c.gaps.len() + c.deliveries.len() + c.service_deliveries.len()
            })
            .sum::<usize>();
        let bytes = serde_json::to_vec(state)
            .map_err(|_| Error::InvalidState)?
            .len();
        if state.chats.len() > self.limits.chats
            || count > self.limits.records
            || bytes
                .checked_mul(4)
                .and_then(|bytes| {
                    self.external
                        .values()
                        .try_fold(bytes, |total, held| total.checked_add(held.bytes))
                })
                .ok_or(Error::Capacity)?
                > self.limits.bytes
        {
            return Err(Error::Capacity);
        }
        Ok(())
    }
    pub fn attempt(&mut self, id: i32, wire_seq: i32) -> Result<()> {
        if self.refused {
            return Err(Error::Storage);
        }
        if self.expired(id, now()?)
            || self.attempts.get(&(id, wire_seq)).copied().unwrap_or(0) >= self.limits.attempts
        {
            self.suspend(id)?;
            return Err(Error::Paused);
        }
        *self.attempts.entry((id, wire_seq)).or_default() += 1;
        Ok(())
    }
}
impl<S: Storage> Storage for TransientStorage<S> {
    fn next_runtime_expiry_ms(&self) -> Result<Option<u64>> {
        let age = (self.limits.retention_seconds * 1000.0).ceil() as u64;
        Ok(self
            .created
            .iter()
            .filter_map(|((id, _), time)| {
                self.state
                    .chats
                    .get(id)
                    .filter(|chat| !chat.paused && chat.state != ChatState::Suspended)
                    .map(|_| time.saturating_add(age))
            })
            .chain(
                self.external
                    .values()
                    .filter(|held| !held.expired)
                    .map(|held| held.created_ms.saturating_add(age)),
            )
            .min())
    }
    fn expire_runtime(&mut self) -> Result<crate::storage::RuntimeExpiry> {
        let timestamp = now()?;
        let age = (self.limits.retention_seconds * 1000.0).ceil() as u64;
        let mut ids = std::collections::BTreeSet::new();
        for ((id, _), created) in &self.created {
            if timestamp >= created.saturating_add(age)
                && self
                    .state
                    .chats
                    .get(id)
                    .is_some_and(|chat| !chat.paused && chat.state != ChatState::Suspended)
            {
                ids.insert(*id);
            }
        }
        let mut unbound = false;
        for held in self.external.values_mut() {
            if !held.expired && timestamp >= held.created_ms.saturating_add(age) {
                held.expired = true;
                match held.chat {
                    Some(id) => {
                        ids.insert(id);
                    }
                    None => unbound = true,
                }
            }
        }
        if ids.is_empty() && !unbound {
            return Ok(crate::storage::RuntimeExpiry::default());
        }
        let mut state = self.state.clone();
        for id in &ids {
            if let Some(chat) = state.chats.get_mut(id) {
                scrub_chat(chat);
            }
        }
        for held in self.external.values_mut() {
            if held.chat.is_some_and(|id| ids.contains(&id)) {
                held.expired = true;
            }
        }
        if unbound {
            suspend_requests(&mut state);
        }
        let persistence_failed = self.persist_suspension(state).is_err();
        Ok(crate::storage::RuntimeExpiry {
            chats: ids.into_iter().collect(),
            unbound,
            persistence_failed,
        })
    }
    fn reserve_external(&mut self, id: Option<i32>, token: u64, bytes: usize) -> Result<()> {
        if self.refused {
            return Err(Error::Storage);
        }
        if token == 0 {
            return Err(Error::InvalidState);
        }
        let held = match self.external.get(&token) {
            Some(previous) if previous.expired => return Err(Error::Paused),
            Some(previous) if previous.chat != id => return Err(Error::InvalidState),
            Some(previous) => ExternalReservation {
                bytes,
                ..previous.clone()
            },
            None => ExternalReservation {
                chat: id,
                bytes,
                created_ms: now()?,
                expired: false,
            },
        };
        let previous = self.external.insert(token, held);
        if let Err(error) = self.check(&self.state) {
            match previous {
                Some(previous) => {
                    self.external.insert(token, previous);
                }
                None => {
                    self.external.remove(&token);
                }
            }
            if let Some(id) = id {
                self.suspend(id)?;
            }
            return Err(error);
        }
        Ok(())
    }
    fn release_external(&mut self, token: u64) {
        self.external.remove(&token);
    }
    fn runtime_refused(
        &mut self,
        id: i32,
        _reason: crate::storage::RuntimeRefusal,
        _lost: bool,
    ) -> Result<()> {
        self.suspend(id)
    }
    fn runtime_policy(&self) -> Option<crate::storage::RuntimePolicy> {
        Some(crate::storage::RuntimePolicy {
            operation_seconds: self.limits.operation_seconds,
            cleanup_seconds: self.limits.cleanup_seconds,
            tasks: self.limits.tasks,
            bytes: self.limits.bytes,
            chats: self.limits.chats,
            records: self.limits.records,
            frame_bytes: self.limits.frame_bytes,
        })
    }
    fn suspend_storage(&mut self) -> Result<()> {
        self.protected.suspend_storage()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        if self.refused {
            return Err(Error::Storage);
        }
        let mut state = self
            .protected
            .resume_storage()?
            .ok_or(Error::InvalidState)?;
        validate_protected(&state)?;
        if state.authorization != self.state.authorization {
            return Err(Error::Authorization);
        }
        // Resume after a completed stop never revives lost volatile queues.
        if self
            .state
            .chats
            .values()
            .any(|chat| chat.state == ChatState::Suspended)
        {
            for chat in state.chats.values_mut() {
                chat.state = ChatState::Suspended;
                chat.paused = true;
            }
        } else {
            return Ok(Some(self.state.clone()));
        }
        self.state = state;
        Ok(Some(self.state.clone()))
    }
    fn note_attempt(&mut self, id: i32, seq: i32) -> Result<()> {
        self.attempt(id, seq)
    }
    fn load(&self) -> Result<Option<State>> {
        if self.refused {
            return Err(Error::Storage);
        }
        Ok(Some(self.state.clone()))
    }
    fn committed_view(&self) -> Option<State> {
        Some(self.state.clone())
    }
    fn admit(&mut self, id: Option<i32>, _operation: Operation, bytes: usize) -> Result<()> {
        self.admitted_chat = None;
        if self.refused {
            return Err(Error::Storage);
        }
        if bytes > self.limits.frame_bytes {
            if let Some(id) = id {
                self.suspend(id)?;
            }
            return Err(Error::Capacity);
        }
        if let Some(id) = id {
            let chat = self
                .state
                .chats
                .get(&id)
                .ok_or(Error::UnknownChat { chat_id: id })?;
            if chat.paused || chat.state == ChatState::Suspended {
                return Err(Error::Paused);
            }
            if self.expired(id, now()?) {
                self.suspend(id)?;
                return Err(Error::Paused);
            }
        }
        self.admitted_chat = id;
        Ok(())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        let admitted_chat = self.admitted_chat.take();
        if self.refused {
            let mut cleanup = self.state.clone();
            for chat in cleanup.chats.values_mut() {
                scrub_chat(chat);
            }
            suspend_requests(&mut cleanup);
            // A recovered backend may finish a stop, but cannot accept any
            // key/counter change, new owner, or renewed participation.
            if serde_json::to_vec(&cleanup).map_err(|_| Error::InvalidState)?
                != serde_json::to_vec(state).map_err(|_| Error::InvalidState)?
            {
                return Err(Error::Storage);
            }
            return self.persist_suspension(cleanup);
        }
        if state.mode != StoreMode::Protected
            || state.authorization != self.state.authorization
            || !state.settings.is_empty()
            || self
                .state
                .chats
                .keys()
                .any(|id| !state.chats.contains_key(id))
        {
            return Err(Error::Paused);
        }
        if let Err(error) = self.check(state) {
            // Refuse an admitted operation on its actual changed owner. Direct
            // storage commits remain atomic, and a no-op admission cannot lend
            // an unrelated or unbound transaction a conversation identity.
            if let Some(id) = admitted_chat
                && let Some(previous) = self.state.chats.get(&id)
                && let Some(next) = state.chats.get(&id)
                && serde_json::to_vec(previous).map_err(|_| Error::InvalidState)?
                    != serde_json::to_vec(next).map_err(|_| Error::InvalidState)?
            {
                self.suspend(id)?;
            }
            return Err(error);
        }
        for (id, old) in &self.state.chats {
            let new = state.chats.get(id).ok_or(Error::Paused)?;
            if old.state == ChatState::Suspended && new.state != ChatState::Suspended {
                return Err(Error::Paused);
            }
            if new.state == ChatState::Closed || old.key.is_some() && new.key.is_none() {
                return Err(Error::Paused);
            }
        }
        let projected = projection(state);
        validate_protected(&projected)?;
        if let Err(error) = self.protected.commit(&projected) {
            self.refused = true;
            return Err(error);
        }
        let timestamp = now()?;
        let mut live = BTreeMap::new();
        for (id, chat) in &state.chats {
            for identity in identities(chat) {
                let key = (*id, identity);
                let created = self.created.get(&key).copied().unwrap_or(timestamp);
                live.insert(key, created);
            }
        }
        self.created = live;
        self.attempts.retain(|(id, seq), _| {
            state
                .chats
                .get(id)
                .is_some_and(|chat| chat.outbox.contains_key(seq))
        });
        self.state = state.clone();
        Ok(())
    }
}

fn scrub_chat(chat: &mut crate::chat::Chat) {
    chat.state = ChatState::Suspended;
    chat.paused = true;
    chat.outbox.clear();
    chat.gaps.clear();
    chat.deliveries.clear();
    chat.service_deliveries.clear();
}
fn projection(state: &State) -> State {
    let mut projected = state.clone();
    for chat in projected.chats.values_mut() {
        chat.outbox.clear();
        chat.gaps.clear();
        chat.deliveries.clear();
        chat.service_deliveries.clear();
    }
    projected
}

pub(crate) fn suspend_requests(state: &mut State) {
    let names = state
        .private_records
        .keys()
        .filter(|name| name.starts_with("pending-create/"))
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        if let Some(record) = state.private_records.remove(&name) {
            state.private_records.insert(
                name.replacen("pending-create/", "suspended-create/", 1),
                record,
            );
        }
    }
}
