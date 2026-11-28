use super::{CustodyLimits, identities, now, usage};
use crate::{
    error::{Error, Result},
    storage::{Operation, SecretRecord, State, Storage, StoreMode},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "../../tests/support_protocol/custody_controls.rs"]
pub(crate) mod review;
const CONTROL: &str = "custody/v1";
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Operator,
    Capacity,
    Age,
    Admission,
    Storage,
    Ingress,
    Deadline,
    Callback,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    pause: Option<PauseReason>,
    lost: bool,
    ages: BTreeMap<String, u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseStatus {
    pub chat_id: i32,
    pub paused: bool,
    pub resumable: bool,
    pub reason: Option<PauseReason>,
    pub records: usize,
    pub bytes: usize,
}
/// A backend wrapper that persists age/control and protocol records together.
pub struct CustodyStorage<S: Storage> {
    backend: S,
    limits: CustodyLimits,
    state: State,
    controls: BTreeMap<i32, Control>,
    expired_runtime: BTreeSet<i32>,
}
impl<S: Storage> CustodyStorage<S> {
    pub fn new(mut backend: S, authorization: String, limits: CustodyLimits) -> Result<Self> {
        limits.validate()?;
        let existing = backend.load()?;
        let mut state = match existing {
            Some(state) => {
                if state.authorization != authorization
                    || !matches!(state.mode, StoreMode::Bounded | StoreMode::Ingress)
                {
                    return Err(Error::InvalidState);
                }
                state
            }
            None => {
                let mut state = State::empty(authorization)?;
                state.mode = StoreMode::Bounded;
                state
            }
        };
        let controls: BTreeMap<i32, Control> = match state.private_records.get(CONTROL) {
            Some(value) => {
                serde_json::from_slice(value.as_bytes()).map_err(|_| Error::InvalidState)?
            }
            None => {
                if !state.chats.is_empty() {
                    return Err(Error::InvalidState);
                }
                BTreeMap::new()
            }
        };
        validate_controls(&state, &controls, limits.records, limits.chats)?;
        save_controls(&mut state, &controls)?;
        backend.commit(&state)?;
        Ok(Self {
            backend,
            limits,
            state,
            controls,
            expired_runtime: BTreeSet::new(),
        })
    }
    pub fn status(&self, id: i32) -> Result<PauseStatus> {
        let chat = self
            .state
            .chats
            .get(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?;
        let control = self.controls.get(&id).cloned().unwrap_or_default();
        let (records, bytes) = usage(chat)?;
        Ok(PauseStatus {
            chat_id: id,
            paused: chat.paused || control.pause.is_some(),
            resumable: !control.lost,
            reason: control.pause,
            records,
            bytes,
        })
    }
    pub fn pause(&mut self, id: i32, reason: PauseReason, lost: bool) -> Result<()> {
        let mut state = self.state.clone();
        let mut controls = self.controls.clone();
        state
            .chats
            .get_mut(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?
            .paused = true;
        let control = controls.entry(id).or_default();
        control.pause.get_or_insert(reason);
        control.lost |= lost;
        save_controls(&mut state, &controls)?;
        self.backend.commit(&state)?;
        self.state = state;
        self.controls = controls;
        self.expired_runtime.remove(&id);
        Ok(())
    }
    pub fn resume(&mut self, id: i32) -> Result<()> {
        let control = self.controls.get(&id).ok_or(Error::InvalidState)?;
        if control.lost {
            return Err(Error::Paused);
        }
        let mut state = self.state.clone();
        let mut controls = self.controls.clone();
        check_chat(
            state
                .chats
                .get(&id)
                .ok_or(Error::UnknownChat { chat_id: id })?,
            control,
            &self.limits,
            now()?,
        )?;
        state
            .chats
            .get_mut(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?
            .paused = false;
        controls.get_mut(&id).ok_or(Error::InvalidState)?.pause = None;
        save_controls(&mut state, &controls)?;
        self.backend.commit(&state)?;
        self.state = state;
        self.controls = controls;
        Ok(())
    }
    pub fn check_custody(&mut self) -> Result<Vec<i32>> {
        let timestamp = now()?;
        let mut paused = Vec::new();
        for id in self.state.chats.keys().copied().collect::<Vec<_>>() {
            let control = self.controls.get(&id).ok_or(Error::InvalidState)?;
            if check_chat(&self.state.chats[&id], control, &self.limits, timestamp).is_err() {
                self.pause(id, PauseReason::Age, false)?;
                paused.push(id);
            }
        }
        Ok(paused)
    }
}
fn save_controls(state: &mut State, controls: &BTreeMap<i32, Control>) -> Result<()> {
    state.private_records.insert(
        CONTROL.into(),
        SecretRecord::new(serde_json::to_vec(controls).map_err(|_| Error::InvalidState)?),
    );
    Ok(())
}
fn validate_controls(
    state: &State,
    controls: &BTreeMap<i32, Control>,
    records: usize,
    chats: usize,
) -> Result<()> {
    if state.chats.len() > chats || controls.len() != state.chats.len() {
        return Err(Error::InvalidState);
    }
    let timestamp = now()?;
    for (id, chat) in &state.chats {
        let control = controls.get(id).ok_or(Error::InvalidState)?;
        if control.ages.len() > records
            || control.lost && control.pause.is_none()
            || control.ages.keys().cloned().collect::<BTreeSet<_>>() != identities(chat)
            || control
                .ages
                .values()
                .any(|age| *age == 0 || *age > timestamp)
        {
            return Err(Error::InvalidState);
        }
    }
    Ok(())
}
fn check_chat(
    chat: &crate::chat::Chat,
    control: &Control,
    limits: &CustodyLimits,
    timestamp: u64,
) -> Result<()> {
    let (records, bytes) = usage(chat)?;
    if records > limits.records || bytes > limits.bytes {
        return Err(Error::Capacity);
    }
    if control
        .ages
        .values()
        .any(|age| timestamp.saturating_sub(*age) as f64 >= limits.age_seconds * 1000.0)
    {
        return Err(Error::Paused);
    }
    Ok(())
}
impl<S: Storage> Storage for CustodyStorage<S> {
    fn admit_budget(&mut self, id: i32, records: usize, bytes: usize) -> Result<()> {
        let chat = self
            .state
            .chats
            .get(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?;
        if chat.paused {
            return Err(Error::Paused);
        }
        let (held_records, held_bytes) = usage(chat)?;
        if held_records
            .checked_add(records)
            .is_none_or(|count| count > self.limits.records)
            || held_bytes
                .checked_add(bytes)
                .is_none_or(|count| count > self.limits.bytes)
        {
            self.pause(id, PauseReason::Capacity, false)?;
            return Err(Error::Capacity);
        }
        Ok(())
    }
    fn next_runtime_expiry_ms(&self) -> Result<Option<u64>> {
        let age = (self.limits.age_seconds * 1000.0).ceil() as u64;
        Ok(self
            .controls
            .iter()
            .filter(|(id, control)| control.pause.is_none() && !self.expired_runtime.contains(id))
            .flat_map(|(_, control)| control.ages.values().map(|time| time.saturating_add(age)))
            .min())
    }
    fn expire_runtime(&mut self) -> Result<crate::storage::RuntimeExpiry> {
        let timestamp = now()?;
        let age = (self.limits.age_seconds * 1000.0).ceil() as u64;
        let chats: Vec<_> = self
            .controls
            .iter()
            .filter(|(id, control)| {
                control.pause.is_none()
                    && !self.expired_runtime.contains(id)
                    && control
                        .ages
                        .values()
                        .any(|time| timestamp >= time.saturating_add(age))
            })
            .map(|(id, _)| *id)
            .collect();
        let mut persistence_failed = false;
        for id in &chats {
            self.expired_runtime.insert(*id);
            persistence_failed |= self.pause(*id, PauseReason::Age, false).is_err();
        }
        Ok(crate::storage::RuntimeExpiry {
            chats,
            unbound: false,
            persistence_failed,
        })
    }
    fn runtime_refused(
        &mut self,
        id: i32,
        reason: crate::storage::RuntimeRefusal,
        mut lost: bool,
    ) -> Result<()> {
        if self.state.mode == StoreMode::Ingress {
            lost = false;
        }
        let reason = match reason {
            crate::storage::RuntimeRefusal::Capacity => PauseReason::Capacity,
            crate::storage::RuntimeRefusal::Deadline => PauseReason::Deadline,
            crate::storage::RuntimeRefusal::Cancelled => PauseReason::Ingress,
            crate::storage::RuntimeRefusal::Callback => PauseReason::Callback,
        };
        self.pause(id, reason, lost)
    }
    fn runtime_policy(&self) -> Option<crate::storage::RuntimePolicy> {
        Some(crate::storage::RuntimePolicy {
            operation_seconds: self.limits.operation_seconds,
            cleanup_seconds: 5.0,
            tasks: self.limits.tasks,
            bytes: self.limits.bytes,
            chats: self.limits.chats,
            records: self.limits.records,
            frame_bytes: self.limits.frame_bytes,
        })
    }
    fn suspend_storage(&mut self) -> Result<()> {
        self.backend.suspend_storage()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        let state = self.backend.resume_storage()?.ok_or(Error::InvalidState)?;
        if state.authorization != self.state.authorization {
            return Err(Error::Authorization);
        }
        let controls = serde_json::from_slice(
            state
                .private_records
                .get(CONTROL)
                .ok_or(Error::InvalidState)?
                .as_bytes(),
        )
        .map_err(|_| Error::InvalidState)?;
        validate_controls(&state, &controls, self.limits.records, self.limits.chats)?;
        self.controls = controls;
        self.state = state;
        Ok(Some(self.state.clone()))
    }
    fn default_auto_save_folder(&self) -> Option<std::path::PathBuf> {
        self.backend.default_auto_save_folder()
    }
    fn mark_nonresumable(&mut self, id: i32) -> Result<()> {
        self.pause(id, PauseReason::Operator, true)
    }
    fn load(&self) -> Result<Option<State>> {
        Ok(Some(self.state.clone()))
    }
    fn committed_view(&self) -> Option<State> {
        self.backend
            .committed_view()
            .or_else(|| Some(self.state.clone()))
    }
    fn admit(&mut self, id: Option<i32>, operation: Operation, bytes: usize) -> Result<()> {
        self.limits.validate()?;
        if bytes > self.limits.frame_bytes {
            if let Some(id) = id {
                self.pause(
                    id,
                    PauseReason::Capacity,
                    operation == Operation::Receive && self.state.mode != StoreMode::Ingress,
                )?;
            }
            return Err(Error::Capacity);
        }
        if let Some(id) = id
            && let Some(chat) = self.state.chats.get(&id)
        {
            let control = self.controls.get(&id).ok_or(Error::InvalidState)?;
            if chat.paused || control.pause.is_some() {
                return Err(Error::Paused);
            }
            if let Err(error) = check_chat(chat, control, &self.limits, now()?) {
                self.pause(
                    id,
                    PauseReason::Age,
                    operation == Operation::Receive && self.state.mode != StoreMode::Ingress,
                )?;
                return Err(error);
            }
        }
        Ok(())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        self.limits.validate()?;
        state.validate()?;
        if !matches!(state.mode, StoreMode::Bounded | StoreMode::Ingress)
            || state.authorization != self.state.authorization
            || state.chats.len() > self.limits.chats
            || self
                .state
                .chats
                .keys()
                .any(|id| !state.chats.contains_key(id))
        {
            return Err(Error::Paused);
        }
        let mut candidate = state.clone();
        let mut controls = self.controls.clone();
        let timestamp = now()?;
        for (id, chat) in &mut candidate.chats {
            let control = controls.entry(*id).or_default();
            let held = identities(chat);
            control.ages.retain(|key, _| held.contains(key));
            for key in held {
                control.ages.entry(key).or_insert(timestamp);
            }
            if chat.paused {
                control.pause.get_or_insert(PauseReason::Operator);
            }
            if !chat.paused {
                if control.lost {
                    return Err(Error::Paused);
                }
                if let Err(error) = check_chat(chat, control, &self.limits, timestamp) {
                    // Loss belongs to newly admitted input in THIS candidate,
                    // never to a previous operation that returned before commit.
                    // Journal-backed ingress retains refused input durably.
                    let lost = self.state.mode != StoreMode::Ingress
                        && self.state.chats.get(id).is_some_and(|previous| {
                            chat.in_seq_no != previous.in_seq_no
                                || chat.gaps.keys().any(|key| !previous.gaps.contains_key(key))
                                || chat
                                    .deliveries
                                    .keys()
                                    .any(|key| !previous.deliveries.contains_key(key))
                                || chat
                                    .service_deliveries
                                    .keys()
                                    .any(|key| !previous.service_deliveries.contains_key(key))
                        });
                    if self.state.chats.contains_key(id) {
                        self.pause(
                            *id,
                            if error == Error::Capacity {
                                PauseReason::Capacity
                            } else {
                                PauseReason::Age
                            },
                            lost,
                        )?;
                    }
                    return Err(error);
                }
                control.pause = None;
            }
        }
        save_controls(&mut candidate, &controls)?;
        validate_controls(
            &candidate,
            &controls,
            self.limits.records,
            self.limits.chats,
        )?;
        self.backend.commit(&candidate)?;
        self.state = self.backend.committed_view().unwrap_or(candidate);
        self.controls = controls;
        Ok(())
    }
}
