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
#[serde(deny_unknown_fields)]
struct State {
    #[serde(deserialize_with = "deserialize_enabled")]
    enabled: BTreeSet<i32>,
    #[serde(deserialize_with = "deserialize_messages")]
    messages: BTreeMap<i32, BTreeMap<i64, SavedSecretMessage>>,
}

// Restrict the stored format without changing the public record types' serde
// behavior. An older reader must refuse fields it cannot preserve on rewrite.
#[derive(Deserialize)]
#[serde(remote = "SavedSecretMessage", deny_unknown_fields)]
struct StoredMessageFields {
    id: i64,
    chat_id: i32,
    sender_id: i64,
    sender_name: String,
    outgoing: bool,
    date_unix: i64,
    text: String,
    reply_to_msg_id: Option<i64>,
    ttl_seconds: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_media")]
    media: Option<SavedMediaRecord>,
}

#[derive(Deserialize)]
#[serde(remote = "SavedMediaRecord", deny_unknown_fields)]
struct StoredMediaFields {
    kind: SecretMediaKind,
    mime_type: String,
    file_name: String,
    size_bytes: u64,
    local_rel_path: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration_secs: Option<u32>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct StoredMedia(#[serde(with = "StoredMediaFields")] SavedMediaRecord);

#[derive(Deserialize)]
#[serde(transparent)]
struct StoredMessage(#[serde(with = "StoredMessageFields")] SavedSecretMessage);

#[derive(Deserialize)]
#[serde(transparent)]
struct StoredChat(
    #[serde(deserialize_with = "deserialize_unique_map")] BTreeMap<i64, StoredMessage>,
);

fn deserialize_media<'de, D>(deserializer: D) -> Result<Option<SavedMediaRecord>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<StoredMedia>::deserialize(deserializer).map(|media| media.map(|record| record.0))
}

fn deserialize_enabled<'de, D>(deserializer: D) -> Result<BTreeSet<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Enabled;

    impl<'de> serde::de::Visitor<'de> for Enabled {
        type Value = BTreeSet<i32>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique enabled auto-save chat identities")
        }

        fn visit_seq<A>(self, mut input: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut enabled = BTreeSet::new();
            while let Some(id) = input.next_element()? {
                if !enabled.insert(id) {
                    return Err(serde::de::Error::custom(
                        "duplicate enabled auto-save identity",
                    ));
                }
            }
            Ok(enabled)
        }
    }

    deserializer.deserialize_seq(Enabled)
}

fn deserialize_messages<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<i32, BTreeMap<i64, SavedSecretMessage>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let chats: BTreeMap<i32, StoredChat> = deserialize_unique_map(deserializer)?;
    Ok(chats
        .into_iter()
        .map(|(chat_id, messages)| {
            let messages = messages
                .0
                .into_iter()
                .map(|(id, message)| (id, message.0))
                .collect();
            (chat_id, messages)
        })
        .collect())
}

fn deserialize_unique_map<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(std::marker::PhantomData<(K, V)>);

    impl<'de, K, V> serde::de::Visitor<'de> for UniqueMap<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique auto-save history identities")
        }

        fn visit_map<A>(self, mut input: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut entries = BTreeMap::new();
            while let Some(id) = input.next_key()? {
                match entries.entry(id) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(input.next_value()?);
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {
                        return Err(serde::de::Error::custom(
                            "duplicate auto-save history identity",
                        ));
                    }
                }
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(UniqueMap(std::marker::PhantomData))
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
