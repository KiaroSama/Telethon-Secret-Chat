use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::errors::SecretChatError;
use crate::ported::rekey::{ExchangeRole, RuntimeRekey};

use super::{ChatRole, ChatState, KeyLifecycleState, KeySlot, SecretChat, key_fingerprint};

fn deserialize_unique_i64_map<'de, D>(deserializer: D) -> Result<BTreeMap<i64, i64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct UniqueMapVisitor;
    impl<'de> Visitor<'de> for UniqueMapVisitor {
        type Value = BTreeMap<i64, i64>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map with unique i64 keys")
        }
        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut out = BTreeMap::new();
            while let Some((k, v)) = access.next_entry::<i64, i64>()? {
                if out.insert(k, v).is_some() {
                    return Err(de::Error::custom("duplicate key in pending_frame_keys"));
                }
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(UniqueMapVisitor)
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistedExchangeRole {
    Requested,
    Accepted,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedProtocolState {
    #[zeroize(skip)]
    pub state: ChatState,
    pub negotiated_layer: i64,
    pub wrapper_layer: i64,
    pub in_seq_no: i64,
    pub out_seq_no: i64,
    pub peer_in_seq_no: i64,
    #[serde(default)]
    pub ttl: i32,
    #[zeroize(skip)]
    pub key_lifecycle: KeyLifecycleState,
    pub active_key: Option<Vec<u8>>,
    pub previous_key: Option<Vec<u8>>,
    pub pending_key: Option<Vec<u8>>,
    pub secret_key: Option<Vec<u8>>,
    #[zeroize(skip)]
    pub dh: Option<(i64, Vec<u8>)>,
    pub exchange: Option<i64>,
    #[zeroize(skip)]
    pub exchange_role: Option<PersistedExchangeRole>,
    pub confirmed: bool,
    pub incoming_key_cutover: Option<i64>,
    pub accepted_exchange: Option<i64>,
    pub pending_noop: bool,
    pub messages: u64,
    pub outgoing: u64,
    #[serde(default)]
    pub rekey_age_seconds: u64,
    #[zeroize(skip)]
    pub pending_frames: Vec<Value>,
    #[zeroize(skip)]
    #[serde(deserialize_with = "deserialize_unique_i64_map")]
    pub pending_frame_keys: BTreeMap<i64, i64>,
    pub gap_requested: bool,
    pub gap_end: Option<i64>,
    #[zeroize(skip)]
    pub resend_due: Option<(i64, i64)>,
    #[zeroize(skip)]
    pub resend_request: Option<(i64, i64)>,
}

fn slot_to_vec(slot: &Option<KeySlot>) -> Option<Vec<u8>> {
    slot.as_ref().map(|s| s.0.to_vec())
}

fn vec_to_slot(chat_id: i64, raw: &Option<Vec<u8>>) -> Result<Option<KeySlot>, SecretChatError> {
    match raw {
        Some(bytes) => {
            let arr: [u8; 256] =
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SecretChatError::StoreCorrupt {
                        chat_id: Some(chat_id),
                        reason: "persisted secret-chat key must be 256 bytes".into(),
                    })?;
            Ok(Some(KeySlot(arr)))
        }
        None => Ok(None),
    }
}

impl PersistedProtocolState {
    /// Apply the live receive invariants before a checkpoint can become live state.
    pub(crate) fn validate(&self, chat_id: i64, role: ChatRole) -> Result<(), SecretChatError> {
        // Wire counters are signed i32; local counts can advance past the final index once.
        const MAX_WIRE_INDEX: i64 = i32::MAX as i64 / 2;
        let invalid = || SecretChatError::StoreCorrupt {
            chat_id: Some(chat_id),
            reason: "inconsistent persisted protocol receive state".into(),
        };
        if self.in_seq_no < 0
            || self.in_seq_no > MAX_WIRE_INDEX + 1
            || self.out_seq_no < 0
            || self.out_seq_no > MAX_WIRE_INDEX + 1
            || self.peer_in_seq_no < 0
            || self.peer_in_seq_no > MAX_WIRE_INDEX
            || self.peer_in_seq_no > self.out_seq_no
            || self.ttl < 0
            || self.wrapper_layer < 46
            || self.negotiated_layer < self.wrapper_layer
            || self.pending_frames.len() != self.pending_frame_keys.len()
        {
            return Err(invalid());
        }
        let outbound = matches!(role, ChatRole::Initiator);
        let state = json!({
            "id": chat_id,
            "is_outbound": outbound,
            "in_seq_no": self.in_seq_no,
            "out_seq_no": self.out_seq_no,
            "peer_in_seq_no": self.peer_in_seq_no,
            "layer": self.negotiated_layer,
            "wrapper_layer": self.wrapper_layer,
        });
        let pending =
            crate::ported::sequence::validate_pending_checkpoint(&state, &self.pending_frames)
                .map_err(|_| invalid())?;
        if !pending.iter().eq(self.pending_frame_keys.keys()) {
            return Err(invalid());
        }
        let held_keys = [&self.active_key, &self.previous_key, &self.pending_key]
            .into_iter()
            .filter_map(|key| key.as_ref())
            .map(|key| key_fingerprint(key).map_err(|_| invalid()))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let preceding_key = self
            .previous_key
            .as_ref()
            .or_else(|| self.pending_key.as_ref().and(self.active_key.as_ref()));
        let preceding = preceding_key
            .map(|key| key_fingerprint(key).map_err(|_| invalid()))
            .transpose()?;
        if self
            .pending_frame_keys
            .iter()
            .any(|(&sequence, fingerprint)| {
                !held_keys.contains(fingerprint)
                    || (preceding.as_ref() == Some(fingerprint)
                        && self
                            .incoming_key_cutover
                            .is_some_and(|cutover| sequence >= cutover))
            })
        {
            return Err(invalid());
        }
        if pending.is_empty() {
            if self.gap_requested
                || self.gap_end.is_some()
                || self.resend_due.is_some()
                || self.resend_request.is_some()
            {
                return Err(invalid());
            }
        } else {
            let end = self
                .gap_end
                .filter(|end| pending.contains(end))
                .ok_or_else(invalid)?;
            let (start_wire, end_wire) = self.resend_due.ok_or_else(invalid)?;
            let parity = i64::from(!outbound);
            if !self.gap_requested
                || start_wire < 0
                || end_wire < start_wire
                || start_wire % 2 != parity
                || end_wire % 2 != parity
                || start_wire / 2 > self.in_seq_no
                || i128::from(end_wire) != (i128::from(end) - 1) * 2 + i128::from(parity)
                || self
                    .resend_request
                    .is_some_and(|span| Some(span) != self.resend_due)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

impl SecretChat {
    pub(crate) fn export_runtime(&self) -> PersistedProtocolState {
        PersistedProtocolState {
            state: self.state,
            negotiated_layer: self.negotiated_layer,
            wrapper_layer: self.wrapper_layer,
            in_seq_no: self.in_seq_no,
            out_seq_no: self.out_seq_no,
            peer_in_seq_no: self.peer_in_seq_no,
            ttl: self.ttl,
            key_lifecycle: self.key_lifecycle,
            active_key: slot_to_vec(&self.active_key),
            previous_key: slot_to_vec(&self.rekey.previous),
            pending_key: slot_to_vec(&self.rekey.pending),
            secret_key: slot_to_vec(&self.rekey.secret),
            dh: self.rekey.dh.clone(),
            exchange: self.rekey.exchange,
            exchange_role: self.rekey.role.map(|role| match role {
                ExchangeRole::Requested => PersistedExchangeRole::Requested,
                ExchangeRole::Accepted => PersistedExchangeRole::Accepted,
            }),
            confirmed: self.rekey.confirmed,
            incoming_key_cutover: self.rekey.incoming_key_cutover,
            accepted_exchange: self.rekey.accepted_exchange,
            pending_noop: self.rekey.pending_noop,
            messages: self.rekey.messages,
            outgoing: self.rekey.outgoing,
            rekey_age_seconds: self.rekey.started.elapsed().as_secs(),
            pending_frames: self.pending_frames.clone(),
            pending_frame_keys: self.pending_frame_keys.clone(),
            gap_requested: self.gap_requested,
            gap_end: self.gap_end,
            resend_due: self.resend_due,
            resend_request: self.resend_request,
        }
    }

    pub(crate) fn import_runtime(
        &mut self,
        rt: &PersistedProtocolState,
    ) -> Result<(), SecretChatError> {
        rt.validate(self.chat_id, self.role)?;
        if rt.in_seq_no < 0 || rt.out_seq_no < 0 || rt.peer_in_seq_no < 0 || rt.ttl < 0 {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(self.chat_id),
                reason: "persisted sequence counters or ttl must be non-negative".into(),
            });
        }
        if rt.negotiated_layer < 46 || rt.wrapper_layer < 46 {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(self.chat_id),
                reason: "persisted protocol layer is out of range".into(),
            });
        }
        let active_key = vec_to_slot(self.chat_id, &rt.active_key)?;
        let previous = vec_to_slot(self.chat_id, &rt.previous_key)?;
        let pending = vec_to_slot(self.chat_id, &rt.pending_key)?;
        let secret = vec_to_slot(self.chat_id, &rt.secret_key)?;
        if matches!(rt.key_lifecycle, KeyLifecycleState::Rekeying { .. })
            && (rt.exchange.is_none() || rt.exchange_role.is_none())
        {
            return Err(SecretChatError::StoreCorrupt {
                chat_id: Some(self.chat_id),
                reason: "incomplete rekeying runtime state cannot be restored".into(),
            });
        }
        self.state = rt.state;
        self.negotiated_layer = rt.negotiated_layer;
        self.wrapper_layer = rt.wrapper_layer;
        self.in_seq_no = rt.in_seq_no;
        self.out_seq_no = rt.out_seq_no;
        self.peer_in_seq_no = rt.peer_in_seq_no;
        self.ttl = rt.ttl;
        self.key_lifecycle = rt.key_lifecycle;
        self.active_key = active_key;
        self.rekey = RuntimeRekey {
            dh: rt.dh.clone(),
            previous,
            pending,
            secret,
            exchange: rt.exchange,
            role: rt.exchange_role.map(|role| match role {
                PersistedExchangeRole::Requested => ExchangeRole::Requested,
                PersistedExchangeRole::Accepted => ExchangeRole::Accepted,
            }),
            confirmed: rt.confirmed,
            incoming_key_cutover: rt.incoming_key_cutover,
            accepted_exchange: rt.accepted_exchange,
            pending_noop: rt.pending_noop,
            messages: rt.messages,
            outgoing: rt.outgoing,
            started: Instant::now()
                .checked_sub(Duration::from_secs(rt.rekey_age_seconds))
                .unwrap_or_else(Instant::now),
        };
        self.pending_frames = rt.pending_frames.clone();
        self.pending_frame_keys = rt.pending_frame_keys.clone();
        self.gap_requested = rt.gap_requested;
        self.gap_end = rt.gap_end;
        self.resend_due = rt.resend_due;
        self.resend_request = rt.resend_request;
        Ok(())
    }
}
