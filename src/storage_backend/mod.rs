//! Account-bound storage for secret-chat checkpoints.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::errors::SecretChatError;
use crate::protocol::{
    ChatRole, ChatState, KeyLifecycleState, MAX_ENCRYPTED_FRAME_LEN, PersistedProtocolState,
    key_fingerprint, outgoing_layer,
};

type Result<T, E = SecretChatError> = std::result::Result<T, E>;

pub const STORE_FORMAT_VERSION: u32 = 1;
const MAX_ACCOUNT_BYTES: usize = 1024;
const MAGIC: &[u8; 8] = b"KSCSTO01";
const PROTECTED_MAGIC: &[u8; 8] = b"KSCPRO01";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct PersistedOutboxEntry {
    pub random_id: i64,
    pub out_seq_no: i32,
    pub payload: Vec<u8>,
    pub attempts: u32,
}

impl fmt::Debug for PersistedOutboxEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedOutboxEntry")
            .field("out_seq_no", &self.out_seq_no)
            .field("payload_bytes", &self.payload.len())
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutboxState {
    pub identity: [u8; 32],
    pub pending: bool,
    pub service: bool,
    pub replay_requested: bool,
    pub acknowledged: bool,
    pub revision: u64,
}

#[derive(Clone, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct PersistedChatRecord {
    pub chat_id: i32,
    pub access_hash: i64,
    pub peer_user_id: i64,
    #[zeroize(skip)]
    pub role: ChatRole,
    #[zeroize(skip)]
    pub state: ChatState,
    pub layer: i32,
    pub ttl: i32,
    #[serde(with = "key_bytes")]
    pub auth_key: [u8; 256],
    pub key_fingerprint: i64,
    pub in_seq_no: i32,
    pub out_seq_no: i32,
    pub sent_since_rekey: u64,
    pub received_since_rekey: u64,
    #[zeroize(skip)]
    pub rekey_state: KeyLifecycleState,
    #[serde(default, with = "optional_key")]
    pub previous_auth_key: Option<[u8; 256]>,
    pub previous_key_max_in_seq: Option<i32>,
    #[zeroize(skip)]
    pub pending_outbox: Vec<PersistedOutboxEntry>,
    pub auto_save_enabled: bool,
    pub tombstoned: bool,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
    #[zeroize(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) protocol: Option<PersistedProtocolState>,
    #[zeroize(skip)]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) outbox: Vec<OutboxState>,
    #[serde(default)]
    pub(crate) checkpoint: [u8; 16],
}

impl fmt::Debug for PersistedChatRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedChatRecord")
            .field("chat_id", &self.chat_id)
            .field("state", &self.state)
            .field("tombstoned", &self.tombstoned)
            .finish_non_exhaustive()
    }
}

impl PersistedChatRecord {
    pub fn established(
        chat_id: i32,
        access_hash: i64,
        peer_user_id: i64,
        role: ChatRole,
        auth_key: [u8; 256],
    ) -> Result<Self> {
        let auth_key = Zeroizing::new(auth_key);
        let fingerprint = key_fingerprint(&*auth_key)?;
        let now = unix_now();
        let record = Self {
            chat_id,
            access_hash,
            peer_user_id,
            role,
            state: ChatState::Ready,
            layer: 46,
            ttl: 0,
            auth_key: *auth_key,
            key_fingerprint: fingerprint,
            in_seq_no: 0,
            out_seq_no: 0,
            sent_since_rekey: 0,
            received_since_rekey: 0,
            rekey_state: KeyLifecycleState::Active { fingerprint },
            previous_auth_key: None,
            previous_key_max_in_seq: None,
            pending_outbox: Vec::new(),
            auto_save_enabled: false,
            tombstoned: false,
            created_at_unix: now,
            updated_at_unix: now,
            protocol: None,
            outbox: Vec::new(),
            checkpoint: [0; 16],
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn tombstone(&mut self) {
        self.auth_key.zeroize();
        self.previous_auth_key.zeroize();
        self.previous_key_max_in_seq = None;
        self.protocol = None;
        self.pending_outbox.clear();
        self.outbox.clear();
        self.key_fingerprint = 0;
        self.rekey_state = KeyLifecycleState::Zeroized;
        self.state = ChatState::Closed;
        self.tombstoned = true;
        self.updated_at_unix = self.updated_at_unix.max(unix_now());
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let invalid = || corrupt("invalid secret-chat checkpoint");
        if self.chat_id <= 0
            || self.peer_user_id <= 0
            || self.ttl < 0
            || self.in_seq_no < 0
            || self.out_seq_no < 0
            || self.created_at_unix < 0
            || self.updated_at_unix < self.created_at_unix
            || outgoing_layer(i64::from(self.layer)).is_err()
        {
            return Err(invalid());
        }
        if self.tombstoned {
            return if self.state == ChatState::Closed
                && self.rekey_state == KeyLifecycleState::Zeroized
                && self.key_fingerprint == 0
                && self.auth_key == [0; 256]
                && self.previous_auth_key.is_none()
                && self.previous_key_max_in_seq.is_none()
                && self.protocol.is_none()
                && self.pending_outbox.is_empty()
                && self.outbox.is_empty()
            {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let fingerprint = match self.rekey_state {
            KeyLifecycleState::Active { fingerprint } => fingerprint,
            KeyLifecycleState::Rekeying { fingerprint, .. } if self.protocol.is_some() => {
                fingerprint
            }
            _ => return Err(invalid()),
        };
        if self.state != ChatState::Ready
            || self.auth_key == [0; 256]
            || key_fingerprint(&self.auth_key)? != self.key_fingerprint
            || fingerprint != self.key_fingerprint
            || self.previous_key_max_in_seq.is_some_and(|seq| {
                seq < -1
                    || (self.previous_auth_key.is_none()
                        && self
                            .protocol
                            .as_ref()
                            .is_none_or(|rt| rt.pending_key.is_none()))
            })
            || self
                .previous_auth_key
                .as_ref()
                .is_some_and(|key| key == &[0; 256] || key == &self.auth_key)
            || self.pending_outbox.len() != self.outbox.len()
        {
            return Err(invalid());
        }
        if let Some(rt) = &self.protocol {
            let cutoff = self.previous_key_max_in_seq.map(|seq| i64::from(seq) + 1);
            if rt.ttl != self.ttl
                || rt.state != self.state
                || rt.key_lifecycle != self.rekey_state
                || rt.negotiated_layer != i64::from(self.layer)
                || rt.in_seq_no != i64::from(self.in_seq_no)
                || rt.out_seq_no != i64::from(self.out_seq_no)
                || rt.active_key.as_deref() != Some(self.auth_key.as_slice())
                || rt.previous_key.as_deref()
                    != self.previous_auth_key.as_ref().map(|key| key.as_slice())
                || rt.incoming_key_cutover != cutoff
                || rt.messages != self.received_since_rekey
                || rt.outgoing != self.sent_since_rekey
            {
                return Err(invalid());
            }
        } else if self.in_seq_no != 0
            || self.out_seq_no != 0
            || self.sent_since_rekey != 0
            || self.received_since_rekey != 0
            || self.previous_auth_key.is_some()
            || !self.pending_outbox.is_empty()
        {
            return Err(invalid());
        }
        let mut last = -1;
        for (entry, meta) in self.pending_outbox.iter().zip(&self.outbox) {
            if entry.out_seq_no <= last
                || entry.out_seq_no >= self.out_seq_no
                || entry.payload.is_empty()
                || entry.payload.len() > MAX_ENCRYPTED_FRAME_LEN
                || (meta.replay_requested && !meta.pending)
            {
                return Err(invalid());
            }
            last = entry.out_seq_no;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedStoreSnapshot {
    #[serde(deserialize_with = "format_version")]
    pub format_version: u32,
    pub account_binding: String,
    #[serde(deserialize_with = "unique_chats")]
    pub chats: BTreeMap<i32, PersistedChatRecord>,
    #[serde(deserialize_with = "unique_suspended")]
    pub suspended_transient_chats: BTreeSet<i32>,
}

impl PersistedStoreSnapshot {
    fn empty(account_binding: String) -> Self {
        Self {
            format_version: STORE_FORMAT_VERSION,
            account_binding,
            chats: BTreeMap::new(),
            suspended_transient_chats: BTreeSet::new(),
        }
    }

    pub(crate) fn validate(&self, account_binding: &str) -> Result<()> {
        validate_binding(account_binding)?;
        if self.format_version != STORE_FORMAT_VERSION
            || !bool::from(
                self.account_binding
                    .as_bytes()
                    .ct_eq(account_binding.as_bytes()),
            )
        {
            return Err(corrupt(
                "secret-chat store version or account binding does not match",
            ));
        }
        for (&id, record) in &self.chats {
            if id != record.chat_id || self.suspended_transient_chats.contains(&id) {
                return Err(corrupt("secret-chat store identities are inconsistent"));
            }
            record.validate()?;
        }
        if self.suspended_transient_chats.iter().any(|&id| id <= 0) {
            return Err(corrupt(
                "secret-chat store contains an invalid transient identity",
            ));
        }
        Ok(())
    }
}

pub trait SecretChatStore: Send + Sync {
    fn load_all(&self) -> Result<PersistedStoreSnapshot>;
    fn get(&self, chat_id: i32) -> Result<Option<PersistedChatRecord>> {
        Ok(self.load_all()?.chats.remove(&chat_id))
    }
    fn save(&self, record: &PersistedChatRecord) -> Result<()>;
    /// Atomically replaces the expected checkpoint, or inserts only if absent.
    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<()>;
    fn delete(&self, chat_id: i32) -> Result<()>;
    fn list_chat_ids(&self) -> Result<Vec<i32>> {
        Ok(self.load_all()?.chats.into_keys().collect())
    }
    fn mark_transient_suspended(&self, chat_id: i32) -> Result<()>;
    fn is_transient_suspended(&self, chat_id: i32) -> Result<bool> {
        Ok(self
            .load_all()?
            .suspended_transient_chats
            .contains(&chat_id))
    }
}

pub struct MemoryStorage(Mutex<PersistedStoreSnapshot>);

impl MemoryStorage {
    pub fn new(account_binding: impl Into<String>) -> Result<Self> {
        let binding = account_binding.into();
        validate_binding(&binding)?;
        Ok(Self(Mutex::new(PersistedStoreSnapshot::empty(binding))))
    }

    fn with<R>(&self, apply: impl FnOnce(&mut PersistedStoreSnapshot) -> R) -> Result<R> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| corrupt("memory store lock was poisoned"))?;
        Ok(apply(&mut state))
    }
}

impl SecretChatStore for MemoryStorage {
    fn load_all(&self) -> Result<PersistedStoreSnapshot> {
        self.with(|state| state.clone())
    }
    fn get(&self, id: i32) -> Result<Option<PersistedChatRecord>> {
        self.with(|state| state.chats.get(&id).cloned())
    }
    fn save(&self, record: &PersistedChatRecord) -> Result<()> {
        self.with(|state| save_record(state, record, None))??;
        Ok(())
    }
    fn save_checkpoint(
        &self,
        expected: Option<[u8; 16]>,
        record: &mut PersistedChatRecord,
    ) -> Result<()> {
        record.checkpoint = self.with(|state| save_record(state, record, Some(expected)))??;
        Ok(())
    }
    fn delete(&self, id: i32) -> Result<()> {
        let _ = self.with(|state| delete_record(state, id))?;
        Ok(())
    }
    fn mark_transient_suspended(&self, id: i32) -> Result<()> {
        let _ = self.with(|state| suspend_record(state, id))?;
        Ok(())
    }
}

mod file_store;
pub use file_store::{FileStorage, ProtectedFileStorage};

fn save_record(
    state: &mut PersistedStoreSnapshot,
    record: &PersistedChatRecord,
    expected: Option<Option<[u8; 16]>>,
) -> Result<[u8; 16]> {
    record.validate()?;
    if state.suspended_transient_chats.contains(&record.chat_id) {
        return Err(SecretChatError::Protocol(
            "transient chat permanently suspended after restart",
        ));
    }
    let existing = state.chats.get(&record.chat_id);
    if !record.tombstoned && existing.is_some_and(|old| old.tombstoned) {
        return Err(corrupt("a tombstoned chat cannot be restored"));
    }
    if let Some(expected_slot) = expected {
        match (existing, expected_slot) {
            (None, None) => {}
            (Some(old), Some(exp)) if bool::from(old.checkpoint.ct_eq(&exp)) => {}
            _ => {
                return Err(corrupt(
                    "secret-chat checkpoint changed concurrently; reopen and reconcile",
                ));
            }
        }
    }
    let mut next = record.clone();
    getrandom::fill(&mut next.checkpoint).map_err(|_| SecretChatError::EntropyUnavailable {
        reason: "OS randomness unavailable".into(),
    })?;
    let checkpoint = next.checkpoint;
    state.chats.insert(record.chat_id, next);
    Ok(checkpoint)
}

fn delete_record(state: &mut PersistedStoreSnapshot, id: i32) -> bool {
    let record = state.chats.remove(&id).is_some();
    state.suspended_transient_chats.remove(&id) || record
}

fn suspend_record(state: &mut PersistedStoreSnapshot, id: i32) -> Result<bool> {
    if id <= 0 || state.chats.contains_key(&id) {
        return Err(corrupt(
            "invalid or already persisted transient chat identity",
        ));
    }
    if !state.suspended_transient_chats.insert(id) {
        return Err(SecretChatError::Protocol(
            "transient chat permanently suspended after restart",
        ));
    }
    Ok(true)
}

fn validate_binding(binding: &str) -> Result<()> {
    if binding.trim().is_empty() || binding.len() > MAX_ACCOUNT_BYTES {
        return Err(corrupt(
            "secret-chat store requires a valid account binding",
        ));
    }
    Ok(())
}

pub(crate) fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) fn corrupt(reason: &'static str) -> SecretChatError {
    SecretChatError::StoreCorrupt {
        chat_id: None,
        reason: reason.into(),
    }
}

fn format_version<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(d)?;
    if version == STORE_FORMAT_VERSION {
        Ok(version)
    } else {
        Err(de::Error::custom("unsupported store format version"))
    }
}

fn unique_chats<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<i32, PersistedChatRecord>, D::Error> {
    struct Chats;
    impl<'de> de::Visitor<'de> for Chats {
        type Value = BTreeMap<i32, PersistedChatRecord>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("unique chat identities")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut chats = BTreeMap::new();
            while let Some((id, record)) = map.next_entry()? {
                if chats.insert(id, record).is_some() {
                    return Err(de::Error::custom("duplicate chat identity"));
                }
            }
            Ok(chats)
        }
    }
    d.deserialize_map(Chats)
}

fn unique_suspended<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeSet<i32>, D::Error> {
    let mut suspended = BTreeSet::new();
    for id in Vec::<i32>::deserialize(d)? {
        if !suspended.insert(id) {
            return Err(de::Error::custom("duplicate transient identity"));
        }
    }
    Ok(suspended)
}

fn decode_key<E: de::Error>(text: String) -> Result<[u8; 256], E> {
    let text = Zeroizing::new(text);
    let mut key = Zeroizing::new([0; 256]);
    hex::decode_to_slice(text.as_bytes(), &mut *key)
        .map_err(|_| E::custom("invalid shared key encoding"))?;
    Ok(*key)
}

mod key_bytes {
    use super::*;
    pub fn serialize<S: Serializer>(key: &[u8; 256], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&Zeroizing::new(hex::encode(key)))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 256], D::Error> {
        decode_key(String::deserialize(d)?)
    }
}

mod optional_key {
    use super::*;
    pub fn serialize<S: Serializer>(
        key: &Option<[u8; 256]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match key {
            Some(key) => serializer.serialize_some(Zeroizing::new(hex::encode(key)).as_str()),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 256]>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(decode_key)
            .transpose()
    }
}
