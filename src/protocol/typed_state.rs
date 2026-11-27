//! Versioned protocol snapshots with explicit gap and rekey phases.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize, de};
use serde_json::Value;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::snapshot::{PersistedExchangeRole, PersistedProtocolState};
use super::{ChatRole, ChatState, KeyLifecycleState, SHARED_KEY_LEN, SecretChat};
use crate::errors::SecretChatError;

pub const TYPED_PROTOCOL_FORMAT_V1: u32 = 1;
pub const TYPED_PROTOCOL_FORMAT_V2: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PersistedGapState {
    None,
    Active {
        gap_end: i64,
        resend_due: Option<(i64, i64)>,
        resend_request: Option<(i64, i64)>,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "phase", deny_unknown_fields)]
pub enum PersistedRekeyExchange {
    Idle {
        #[zeroize(skip)]
        dh: Option<(i64, Vec<u8>)>,
        previous_key: Option<Vec<u8>>,
        confirmed: bool,
        incoming_key_cutover: Option<i64>,
        accepted_exchange: Option<i64>,
        pending_noop: bool,
    },
    Requested {
        exchange: i64,
        #[zeroize(skip)]
        dh: Option<(i64, Vec<u8>)>,
        secret_key: Vec<u8>,
        previous_key: Option<Vec<u8>>,
        pending_key: Option<Vec<u8>>,
        confirmed: bool,
        incoming_key_cutover: Option<i64>,
        accepted_exchange: Option<i64>,
        pending_noop: bool,
    },
    Accepted {
        exchange: i64,
        #[zeroize(skip)]
        dh: Option<(i64, Vec<u8>)>,
        pending_key: Vec<u8>,
        previous_key: Option<Vec<u8>>,
        confirmed: bool,
        incoming_key_cutover: Option<i64>,
        accepted_exchange: Option<i64>,
        pending_noop: bool,
    },
}

impl fmt::Debug for PersistedRekeyExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let phase = match self {
            Self::Idle { .. } => "Idle",
            Self::Requested { .. } => "Requested",
            Self::Accepted { .. } => "Accepted",
        };
        f.debug_struct("PersistedRekeyExchange")
            .field("phase", &phase)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct TypedPersistedProtocol {
    pub format_version: u32,
    #[zeroize(skip)]
    pub state: ChatState,
    pub negotiated_layer: i64,
    pub wrapper_layer: i64,
    pub in_seq_no: i64,
    pub out_seq_no: i64,
    pub peer_in_seq_no: i64,
    pub ttl: i32,
    #[zeroize(skip)]
    pub key_lifecycle: KeyLifecycleState,
    pub active_key: Option<Vec<u8>>,
    pub rekey: PersistedRekeyExchange,
    pub messages: u64,
    pub outgoing: u64,
    pub rekey_age_seconds: u64,
    #[zeroize(skip)]
    pub pending_frames: Vec<Value>,
    #[zeroize(skip)]
    pub pending_frame_keys: BTreeMap<i64, i64>,
    #[zeroize(skip)]
    pub gap: PersistedGapState,
}

impl fmt::Debug for TypedPersistedProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedPersistedProtocol")
            .field("format_version", &self.format_version)
            .field("state", &self.state)
            .field("in_seq_no", &self.in_seq_no)
            .field("out_seq_no", &self.out_seq_no)
            .field("gap", &self.gap)
            .field("rekey", &self.rekey)
            .finish_non_exhaustive()
    }
}

fn corrupt(reason: &str) -> SecretChatError {
    SecretChatError::StoreCorrupt {
        chat_id: None,
        reason: reason.into(),
    }
}

fn validate_flat(flat: &PersistedProtocolState) -> Result<(), SecretChatError> {
    // Standalone snapshots lack the chat role. Import checks the actual role again.
    if flat.validate(0, ChatRole::Initiator).is_err()
        && flat.validate(0, ChatRole::Acceptor).is_err()
    {
        return Err(corrupt("inconsistent persisted protocol receive state"));
    }
    if [
        &flat.active_key,
        &flat.previous_key,
        &flat.pending_key,
        &flat.secret_key,
    ]
    .into_iter()
    .flatten()
    .any(|key| key.len() != SHARED_KEY_LEN)
    {
        return Err(corrupt("invalid persisted protocol key length"));
    }
    let expected_fingerprint = match flat.key_lifecycle {
        KeyLifecycleState::Active { fingerprint }
        | KeyLifecycleState::Rekeying { fingerprint, .. } => Some(fingerprint),
        KeyLifecycleState::Unset | KeyLifecycleState::Zeroized => None,
    };
    let fingerprint = flat
        .active_key
        .as_deref()
        .map(super::key_fingerprint)
        .transpose()?;
    if fingerprint != expected_fingerprint
        || (flat.state == ChatState::Ready) != fingerprint.is_some()
    {
        return Err(corrupt("inconsistent persisted active key lifecycle"));
    }
    let phase_valid = match (flat.exchange, flat.exchange_role, flat.key_lifecycle) {
        (None, None, lifecycle) => {
            !matches!(lifecycle, KeyLifecycleState::Rekeying { .. })
                && flat.secret_key.is_none()
                && flat.pending_key.is_none()
        }
        (Some(exchange), Some(role), KeyLifecycleState::Rekeying { exchange_id, .. }) => {
            exchange == exchange_id
                && flat.active_key.is_some()
                && match role {
                    PersistedExchangeRole::Requested => flat.secret_key.is_some(),
                    PersistedExchangeRole::Accepted => {
                        flat.secret_key.is_none() && flat.pending_key.is_some()
                    }
                }
        }
        _ => false,
    };
    if !phase_valid {
        return Err(corrupt("inconsistent persisted rekey phase"));
    }
    Ok(())
}

// The flat checkpoint is internal; the public compatibility boundary is JSON.
#[allow(private_interfaces)]
impl TypedPersistedProtocol {
    pub fn from_flat(flat: &PersistedProtocolState) -> Result<Self, SecretChatError> {
        validate_flat(flat)?;
        let gap = flat.gap_end.map_or(PersistedGapState::None, |gap_end| {
            PersistedGapState::Active {
                gap_end,
                resend_due: flat.resend_due,
                resend_request: flat.resend_request,
            }
        });
        let missing_key = || corrupt("missing required rekey key");
        let rekey = match (flat.exchange, flat.exchange_role) {
            (Some(exchange), Some(PersistedExchangeRole::Requested)) => {
                PersistedRekeyExchange::Requested {
                    exchange,
                    dh: flat.dh.clone(),
                    secret_key: flat.secret_key.clone().ok_or_else(missing_key)?,
                    previous_key: flat.previous_key.clone(),
                    pending_key: flat.pending_key.clone(),
                    confirmed: flat.confirmed,
                    incoming_key_cutover: flat.incoming_key_cutover,
                    accepted_exchange: flat.accepted_exchange,
                    pending_noop: flat.pending_noop,
                }
            }
            (Some(exchange), Some(PersistedExchangeRole::Accepted)) => {
                PersistedRekeyExchange::Accepted {
                    exchange,
                    dh: flat.dh.clone(),
                    pending_key: flat.pending_key.clone().ok_or_else(missing_key)?,
                    previous_key: flat.previous_key.clone(),
                    confirmed: flat.confirmed,
                    incoming_key_cutover: flat.incoming_key_cutover,
                    accepted_exchange: flat.accepted_exchange,
                    pending_noop: flat.pending_noop,
                }
            }
            _ => PersistedRekeyExchange::Idle {
                dh: flat.dh.clone(),
                previous_key: flat.previous_key.clone(),
                confirmed: flat.confirmed,
                incoming_key_cutover: flat.incoming_key_cutover,
                accepted_exchange: flat.accepted_exchange,
                pending_noop: flat.pending_noop,
            },
        };
        Ok(Self {
            format_version: TYPED_PROTOCOL_FORMAT_V2,
            state: flat.state,
            negotiated_layer: flat.negotiated_layer,
            wrapper_layer: flat.wrapper_layer,
            in_seq_no: flat.in_seq_no,
            out_seq_no: flat.out_seq_no,
            peer_in_seq_no: flat.peer_in_seq_no,
            ttl: flat.ttl,
            key_lifecycle: flat.key_lifecycle,
            active_key: flat.active_key.clone(),
            rekey,
            messages: flat.messages,
            outgoing: flat.outgoing,
            rekey_age_seconds: flat.rekey_age_seconds,
            pending_frames: flat.pending_frames.clone(),
            pending_frame_keys: flat.pending_frame_keys.clone(),
            gap,
        })
    }

    pub fn to_flat(&self) -> Result<PersistedProtocolState, SecretChatError> {
        if self.format_version != TYPED_PROTOCOL_FORMAT_V2 {
            return Err(corrupt("unexpected typed protocol format version"));
        }
        let (gap_end, resend_due, resend_request) = match self.gap {
            PersistedGapState::None => (None, None, None),
            PersistedGapState::Active {
                gap_end,
                resend_due,
                resend_request,
            } => (Some(gap_end), resend_due, resend_request),
        };
        let (dh, previous_key, confirmed, incoming_key_cutover, accepted_exchange, pending_noop) =
            match &self.rekey {
                PersistedRekeyExchange::Idle {
                    dh,
                    previous_key,
                    confirmed,
                    incoming_key_cutover,
                    accepted_exchange,
                    pending_noop,
                }
                | PersistedRekeyExchange::Requested {
                    dh,
                    previous_key,
                    confirmed,
                    incoming_key_cutover,
                    accepted_exchange,
                    pending_noop,
                    ..
                }
                | PersistedRekeyExchange::Accepted {
                    dh,
                    previous_key,
                    confirmed,
                    incoming_key_cutover,
                    accepted_exchange,
                    pending_noop,
                    ..
                } => (
                    dh,
                    previous_key,
                    *confirmed,
                    *incoming_key_cutover,
                    *accepted_exchange,
                    *pending_noop,
                ),
            };
        let (exchange, exchange_role, pending_key, secret_key) = match &self.rekey {
            PersistedRekeyExchange::Idle { .. } => (None, None, None, None),
            PersistedRekeyExchange::Requested {
                exchange,
                pending_key,
                secret_key,
                ..
            } => (
                Some(*exchange),
                Some(PersistedExchangeRole::Requested),
                pending_key.clone(),
                Some(secret_key.clone()),
            ),
            PersistedRekeyExchange::Accepted {
                exchange,
                pending_key,
                ..
            } => (
                Some(*exchange),
                Some(PersistedExchangeRole::Accepted),
                Some(pending_key.clone()),
                None,
            ),
        };
        let flat = PersistedProtocolState {
            state: self.state,
            negotiated_layer: self.negotiated_layer,
            wrapper_layer: self.wrapper_layer,
            in_seq_no: self.in_seq_no,
            out_seq_no: self.out_seq_no,
            peer_in_seq_no: self.peer_in_seq_no,
            ttl: self.ttl,
            key_lifecycle: self.key_lifecycle,
            active_key: self.active_key.clone(),
            dh: dh.clone(),
            previous_key: previous_key.clone(),
            pending_key,
            secret_key,
            exchange,
            exchange_role,
            confirmed,
            incoming_key_cutover,
            accepted_exchange,
            pending_noop,
            messages: self.messages,
            outgoing: self.outgoing,
            rekey_age_seconds: self.rekey_age_seconds,
            pending_frames: self.pending_frames.clone(),
            pending_frame_keys: self.pending_frame_keys.clone(),
            gap_requested: gap_end.is_some(),
            gap_end,
            resend_due,
            resend_request,
        };
        validate_flat(&flat)?;
        Ok(flat)
    }
}

// Check every object, including opaque frame JSON, before decoding the typed record.
struct UniqueJson;

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = UniqueJson;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_unit<E: de::Error>(self) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<UniqueJson, A::Error> {
                while seq.next_element::<UniqueJson>()?.is_some() {}
                Ok(UniqueJson)
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<UniqueJson, A::Error> {
                let mut seen = std::collections::BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    map.next_value::<UniqueJson>()?;
                }
                Ok(UniqueJson)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Migrates the original unversioned flat record or an explicitly tagged v1 record.
pub fn migrate_json_v1_to_v2(v1_json: &str) -> Result<String, SecretChatError> {
    let invalid = |_| corrupt("invalid flat protocol JSON");
    serde_json::from_str::<UniqueJson>(v1_json).map_err(invalid)?;
    let mut value: Value = serde_json::from_str(v1_json).map_err(invalid)?;
    let fields = value
        .as_object_mut()
        .ok_or_else(|| corrupt("expected flat protocol object"))?;
    if fields
        .remove("format_version")
        .is_some_and(|version| version.as_u64() != Some(u64::from(TYPED_PROTOCOL_FORMAT_V1)))
    {
        return Err(corrupt("unexpected flat protocol format version"));
    }
    let flat: PersistedProtocolState = serde_json::from_value(value).map_err(invalid)?;
    serde_json::to_string(&TypedPersistedProtocol::from_flat(&flat)?)
        .map_err(|_| corrupt("could not serialize typed protocol"))
}

/// Decodes and validates a v2 protocol snapshot. Error messages never include input bytes.
pub fn decode_v2_json(v2_json: &str) -> Result<TypedPersistedProtocol, SecretChatError> {
    let invalid = |_| corrupt("invalid typed protocol JSON");
    serde_json::from_str::<UniqueJson>(v2_json).map_err(invalid)?;
    let typed: TypedPersistedProtocol = serde_json::from_str(v2_json).map_err(invalid)?;
    typed.to_flat()?;
    Ok(typed)
}

impl SecretChat {
    pub fn export_typed_runtime(&self) -> Result<TypedPersistedProtocol, SecretChatError> {
        TypedPersistedProtocol::from_flat(&self.export_runtime())
    }

    pub fn import_typed_runtime(
        &mut self,
        typed: &TypedPersistedProtocol,
    ) -> Result<(), SecretChatError> {
        self.import_runtime(&typed.to_flat()?)
    }
}
