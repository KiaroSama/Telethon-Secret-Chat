//! Opt-in encrypted local history, separate from protocol chat state.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Component, PathBuf},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    errors::SecretChatError,
    media::{SecretMediaKind, parameter},
    storage_file,
};

const MAGIC: &[u8; 8] = b"KSCAUT01";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = MAGIC.len() + NONCE_LEN;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SavedMediaRecord {
    pub kind: SecretMediaKind,
    pub mime_type: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub local_rel_path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_secs: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SavedSecretMessage {
    pub id: i64,
    pub chat_id: i32,
    pub sender_id: i64,
    pub sender_name: String,
    pub outgoing: bool,
    pub date_unix: i64,
    pub text: String,
    pub reply_to_msg_id: Option<i64>,
    pub ttl_seconds: Option<i32>,
    pub media: Option<SavedMediaRecord>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct State {
    enabled: BTreeSet<i32>,
    messages: BTreeMap<i32, BTreeMap<i64, SavedSecretMessage>>,
}

/// Holds an exclusive lock for the lifetime of the store. The caller must use
/// a private directory and supply an independent, securely stored 32-byte key.
pub struct AutoSaveStore {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    _lock: File,
    state: State,
    uncertain: bool,
}

impl AutoSaveStore {
    pub fn open(store_path: PathBuf, key: [u8; 32]) -> Result<Self, SecretChatError> {
        let key = Zeroizing::new(key);
        if store_path
            .components()
            .any(|part| part == Component::ParentDir)
        {
            return Err(parameter(
                "auto-save path must not contain a parent component",
            ));
        }
        let path = storage_file::absolute_store_path(&store_path).map_err(|_| {
            parameter("auto-save requires an absolute file path in an existing directory")
        })?;
        let lock = storage_file::lock_store(&path)
            .map_err(|_| store_error("could not lock auto-save store"))?;
        let state = match storage_file::read_store(&path)
            .map_err(|_| store_error("could not read auto-save store"))?
        {
            Some(raw) => decode(&raw, &key)?,
            None => State::default(),
        };
        Ok(Self {
            path,
            key,
            _lock: lock,
            state,
            uncertain: false,
        })
    }

    pub fn start_auto_save(&mut self, chat_id: i32) -> Result<(), SecretChatError> {
        self.set_enabled(chat_id, true)
    }

    /// Disables future recording and leaves the saved messages in place.
    pub fn stop_auto_save(&mut self, chat_id: i32) -> Result<(), SecretChatError> {
        self.set_enabled(chat_id, false)
    }

    pub fn is_auto_save_enabled(&self, chat_id: i32) -> bool {
        self.state.enabled.contains(&chat_id)
    }

    /// Returns `false` for disabled chats or messages already in this history.
    pub fn record_message(&mut self, msg: SavedSecretMessage) -> Result<bool, SecretChatError> {
        if !self.is_auto_save_enabled(msg.chat_id)
            || self
                .state
                .messages
                .get(&msg.chat_id)
                .is_some_and(|chat| chat.contains_key(&msg.id))
        {
            return Ok(false);
        }
        let mut next = self.state.clone();
        next.messages
            .entry(msg.chat_id)
            .or_default()
            .insert(msg.id, msg);
        self.persist(next)?;
        Ok(true)
    }

    /// Returns at most `limit` messages, most recent first.
    pub fn read_saved_messages(&self, chat_id: i32, limit: usize) -> Vec<SavedSecretMessage> {
        let Some(chat) = self.state.messages.get(&chat_id) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }
        let mut ordered: Vec<_> = chat.values().collect();
        ordered.sort_unstable_by_key(|msg| Reverse((msg.date_unix, msg.id)));
        ordered.into_iter().take(limit).cloned().collect()
    }

    /// Pass `None` to remove the entire history, or `Some` for selected IDs.
    /// This does not change whether the chat is enabled or remove media files.
    pub fn delete_saved_messages(
        &mut self,
        chat_id: i32,
        message_ids: Option<&[i64]>,
    ) -> Result<usize, SecretChatError> {
        if !self.state.messages.contains_key(&chat_id) {
            return Ok(0);
        }
        let mut next = self.state.clone();
        let Some(chat) = next.messages.get_mut(&chat_id) else {
            return Ok(0);
        };
        let removed = match message_ids {
            None => {
                let count = chat.len();
                chat.clear();
                count
            }
            Some(ids) => ids.iter().filter(|id| chat.remove(id).is_some()).count(),
        };
        if removed != 0 {
            if chat.is_empty() {
                next.messages.remove(&chat_id);
            }
            self.persist(next)?;
        }
        Ok(removed)
    }

    /// Chat IDs with retained messages, whether or not recording is enabled.
    pub fn list_saved_chats(&self) -> Vec<i32> {
        self.state.messages.keys().copied().collect()
    }

    fn set_enabled(&mut self, chat_id: i32, enabled: bool) -> Result<(), SecretChatError> {
        if self.is_auto_save_enabled(chat_id) == enabled {
            return Ok(());
        }
        let mut next = self.state.clone();
        if enabled {
            next.enabled.insert(chat_id);
        } else {
            next.enabled.remove(&chat_id);
        }
        self.persist(next)
    }

    fn persist(&mut self, next: State) -> Result<(), SecretChatError> {
        if self.uncertain {
            return Err(store_error(
                "reopen the auto-save store after an uncertain write",
            ));
        }
        let raw = encode(&next, &self.key)?;
        match storage_file::atomic_replace(&self.path, &raw) {
            Ok(()) => {
                self.state = next;
                Ok(())
            }
            Err(failure) if failure.replaced => {
                self.state = next;
                self.uncertain = true;
                Err(store_error(
                    "auto-save replacement durability could not be confirmed",
                ))
            }
            Err(_) => Err(store_error("could not replace auto-save store")),
        }
    }
}

fn encode(state: &State, key: &[u8; 32]) -> Result<Vec<u8>, SecretChatError> {
    let payload = Zeroizing::new(
        serde_json::to_vec(state)
            .map_err(|_| store_error("could not serialize auto-save history"))?,
    );
    if payload.len() > storage_file::MAX_STORE_BYTES - HEADER_LEN - TAG_LEN {
        return Err(store_error(
            "auto-save history exceeds the store size limit",
        ));
    }
    let mut nonce = [0; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| SecretChatError::EntropyUnavailable {
        reason: "could not generate an auto-save nonce".into(),
    })?;
    let ciphertext = cipher(key)?
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &payload,
                aad: MAGIC,
            },
        )
        .map_err(|_| store_error("could not encrypt auto-save history"))?;
    let mut raw = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    raw.extend_from_slice(MAGIC);
    raw.extend_from_slice(&nonce);
    raw.extend_from_slice(&ciphertext);
    Ok(raw)
}

fn decode(raw: &[u8], key: &[u8; 32]) -> Result<State, SecretChatError> {
    if raw.len() < HEADER_LEN + TAG_LEN || !raw.starts_with(MAGIC) {
        return Err(store_error("invalid auto-save file header"));
    }
    let nonce: [u8; NONCE_LEN] = raw[MAGIC.len()..HEADER_LEN]
        .try_into()
        .map_err(|_| store_error("invalid auto-save nonce"))?;
    let payload = Zeroizing::new(
        cipher(key)?
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &raw[HEADER_LEN..],
                    aad: MAGIC,
                },
            )
            .map_err(|_| store_error("could not authenticate auto-save history"))?,
    );
    let state: State =
        serde_json::from_slice(&payload).map_err(|_| store_error("invalid auto-save history"))?;
    if state.messages.iter().any(|(&chat_id, messages)| {
        messages.is_empty()
            || messages
                .iter()
                .any(|(&id, message)| message.chat_id != chat_id || message.id != id)
    }) {
        return Err(store_error("inconsistent auto-save history index"));
    }
    Ok(state)
}

fn cipher(key: &[u8; 32]) -> Result<Aes256Gcm, SecretChatError> {
    Aes256Gcm::new_from_slice(key).map_err(|_| store_error("invalid auto-save key length"))
}

fn store_error(reason: &str) -> SecretChatError {
    SecretChatError::StoreCorrupt {
        chat_id: None,
        reason: reason.into(),
    }
}
