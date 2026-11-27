//! The Rust port of `rust/adapters/rekey_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use serde_json::Value;
use serde_json::json;
use sha1::{Digest, Sha1};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use crate::errors::SecretChatError;
use crate::protocol::{
    ChatState, KeyLifecycleState, KeySlot, PreparedServiceMessage, SecretChat, check_dh_config,
    check_peer_value, dh_public_value, dh_shared_key, encrypt_frame, generate_secret_bytes,
    key_fingerprint, outgoing_layer, verify_dh_fingerprint,
};

const REQUEST: u32 = 0xf3c9611b;
const ACCEPT: u32 = 0x6fe1735b;
const COMMIT: u32 = 0xec2e0b9b;
const ABORT: u32 = 0xdd05ec6b;
const NOOP: u32 = 0xa82fdd63;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExchangeRole {
    Requested,
    Accepted,
}

#[derive(Clone)]
pub(crate) struct RuntimeRekey {
    pub(crate) dh: Option<(i64, Vec<u8>)>,
    pub(crate) previous: Option<KeySlot>,
    pub(crate) pending: Option<KeySlot>,
    pub(crate) secret: Option<KeySlot>,
    pub(crate) exchange: Option<i64>,
    pub(crate) role: Option<ExchangeRole>,
    pub(crate) confirmed: bool,
    pub(crate) pending_noop: bool,
    pub(crate) messages: u64,
    pub(crate) outgoing: u64,
    pub(crate) started: Instant,
}

impl Default for RuntimeRekey {
    fn default() -> Self {
        Self {
            dh: None,
            previous: None,
            pending: None,
            secret: None,
            exchange: None,
            role: None,
            confirmed: false,
            pending_noop: false,
            messages: 0,
            outgoing: 0,
            started: Instant::now(),
        }
    }
}

impl SecretChat {
    /// Store the validated parameters of the original exchange for later rekeying.
    pub fn configure_dh(&mut self, g: i64, p: &[u8]) -> Result<(), SecretChatError> {
        self.require_rekey_ready()?;
        check_dh_config(g, p, Some(self.chat_id()))?;
        if self
            .rekey
            .dh
            .as_ref()
            .is_some_and(|(old_g, old_p)| *old_g != g || old_p != p)
        {
            return Err(self.rekey_rejected("the original DH parameters cannot change"));
        }
        self.rekey.dh = Some((g, p.to_vec()));
        Ok(())
    }

    pub fn rekey_due(&self) -> bool {
        self.state() == ChatState::Ready
            && self.rekey.exchange.is_none()
            && self.rekey.previous.is_none()
            && self.rekey.outgoing > 0
            && (self.rekey.messages > 100
                || self.rekey.started.elapsed() > Duration::from_secs(7 * 24 * 60 * 60))
    }

    /// A pending-key packet can confirm a lost CommitKey; acknowledge it with Noop.
    pub fn take_pending_noop(&mut self) -> bool {
        std::mem::take(&mut self.rekey.pending_noop)
    }

    /// Reserve a sequence position and retain the returned ciphertext before transport I/O.
    pub fn prepare_service_action(
        &mut self,
        action: &Value,
    ) -> Result<PreparedServiceMessage, SecretChatError> {
        self.require_rekey_ready()?;
        let key = Zeroizing::new(
            self.active_key
                .as_ref()
                .ok_or_else(|| self.rekey_rejected("the chat has no active key"))?
                .0,
        );
        self.prepare_service_with_key(action, &key)
    }

    fn prepare_service_with_key(
        &mut self,
        action: &Value,
        key: &[u8; 256],
    ) -> Result<PreparedServiceMessage, SecretChatError> {
        self.require_rekey_ready()?;
        let wire = |raw: i64, parity: bool| {
            raw.checked_mul(2)
                .and_then(|n| n.checked_add(i64::from(parity)))
                .filter(|&n| i32::try_from(n).is_ok() && n >= 0)
                .ok_or_else(|| self.rekey_rejected("the secret-chat sequence counter is exhausted"))
        };
        let input = wire(self.in_seq_no(), !self.is_outbound())?;
        let output = wire(self.out_seq_no(), self.is_outbound())?;
        let mut random = [0u8; 31];
        getrandom::fill(&mut random).map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "OS randomness unavailable".into(),
        })?;
        let random_id = exchange_id().map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "OS randomness unavailable".into(),
        })?;
        let body = super::schema_secret_tl::encode_object(json!({"id":0x1be31789_u32,"fields":{
            "random_bytes":{"hex":hex::encode(random)},"layer":outgoing_layer(self.negotiated_layer())?,
            "in_seq_no":input,"out_seq_no":output,
            "message":{"id":0x73164160_u32,"fields":{"random_id":random_id,"action":action}}
        }})).map_err(|_| self.rekey_rejected("invalid service action"))?;
        let body = Zeroizing::new(
            hex::decode(body).map_err(|_| self.rekey_rejected("invalid service encoding"))?,
        );
        let frame = encrypt_frame(key, &body, self.out_x())?;
        if frame.len() > crate::protocol::MAX_ENCRYPTED_FRAME_LEN {
            return Err(
                self.rekey_rejected("the encrypted service frame exceeds the local size limit")
            );
        }
        let prepared = PreparedServiceMessage {
            random_id,
            out_seq_no: self.out_seq_no(),
            frame,
        };
        self.set_out_seq_no(self.out_seq_no() + 1);
        Ok(prepared)
    }

    pub fn start_rekey(&mut self) -> Result<Option<PreparedServiceMessage>, SecretChatError> {
        self.require_rekey_ready()?;
        if self.rekey.exchange.is_some() || self.rekey.previous.is_some() {
            return Ok(None);
        }
        let (g, p) = self.required_dh()?;
        let secret = new_secret()?;
        let public = checked_public(g, &secret, &p, self.chat_id())?;
        let id = exchange_id().map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "OS randomness unavailable".into(),
        })?;
        let prepared = self.prepare_service_action(
            &json!({"id":REQUEST,"fields":{"exchange_id":id,"g_a":{"hex":hex::encode(public)}}}),
        )?;
        self.rekey.exchange = Some(id);
        self.rekey.secret = Some(KeySlot(*secret));
        self.rekey.role = Some(ExchangeRole::Requested);
        self.mark_rekeying(id)?;
        Ok(Some(prepared))
    }

    /// Apply an authenticated, sequence-ordered service action and return exact replies.
    pub fn handle_rekey_action(
        &mut self,
        action: &Value,
    ) -> Result<Vec<PreparedServiceMessage>, SecretChatError> {
        self.require_rekey_ready()?;
        let id = action["id"].as_u64().and_then(|id| u32::try_from(id).ok());
        if !matches!(id, Some(REQUEST | ACCEPT | COMMIT | ABORT | NOOP)) {
            return Ok(Vec::new());
        }
        let encoded = super::schema_secret_tl::encode_object(action.clone())
            .map_err(|_| self.rekey_rejected("invalid rekey action"))?;
        super::schema_secret_tl::decode_object(encoded, "DecryptedMessageAction".into(), 0)
            .map_err(|_| self.rekey_rejected("invalid rekey action"))?;
        if id == Some(NOOP) {
            return Ok(Vec::new());
        }
        let exchange = action["fields"]["exchange_id"]
            .as_i64()
            .ok_or_else(|| self.rekey_rejected("invalid exchange identifier"))?;
        match id {
            Some(REQUEST) => self.handle_key_request(exchange, action),
            Some(ACCEPT) => self.handle_key_accept(exchange, action),
            Some(COMMIT) => self.handle_key_commit(exchange, action),
            Some(ABORT) if self.rekey.exchange == Some(exchange) => {
                self.clear_rekey_exchange()?;
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn handle_key_request(
        &mut self,
        exchange: i64,
        action: &Value,
    ) -> Result<Vec<PreparedServiceMessage>, SecretChatError> {
        if self.rekey.previous.is_some() {
            return self.abort_remote_exchange(exchange);
        }
        if let Some(current) = self.rekey.exchange {
            if self.rekey.role != Some(ExchangeRole::Requested) || current > exchange {
                return Ok(Vec::new());
            }
            if current == exchange {
                self.clear_rekey_exchange()?;
                return Ok(Vec::new());
            }
        }
        let (g, p) = self.required_dh()?;
        let peer = peer_bytes(action, "g_a").map_err(|_| {
            self.close();
            self.rekey_rejected("invalid peer DH value")
        })?;
        let secret = new_secret()?;
        let key = match dh_shared_key(&peer, &*secret, &p, Some(self.chat_id())) {
            Ok(key) => Zeroizing::new(key),
            Err(error) => {
                self.close();
                return Err(error);
            }
        };
        let public = checked_public(g, &secret, &p, self.chat_id())?;
        let prepared = self.prepare_service_action(&json!({"id":ACCEPT,"fields":{
            "exchange_id":exchange,"g_b":{"hex":hex::encode(public)},"key_fingerprint":key_fingerprint(&*key)?
        }}))?;
        self.rekey.exchange = Some(exchange);
        self.rekey.pending = Some(KeySlot(*key));
        self.rekey.secret = None;
        self.rekey.role = Some(ExchangeRole::Accepted);
        self.mark_rekeying(exchange)?;
        Ok(vec![prepared])
    }

    fn handle_key_accept(
        &mut self,
        exchange: i64,
        action: &Value,
    ) -> Result<Vec<PreparedServiceMessage>, SecretChatError> {
        if self.rekey.exchange != Some(exchange) || self.rekey.role != Some(ExchangeRole::Requested)
        {
            return Ok(Vec::new());
        }
        let (_, p) = self.required_dh()?;
        let secret = Zeroizing::new(
            self.rekey
                .secret
                .as_ref()
                .ok_or_else(|| self.rekey_rejected("missing exchange secret"))?
                .0,
        );
        let peer = match peer_bytes(action, "g_b") {
            Ok(peer) => peer,
            Err(_) => {
                let response = self.abort_remote_exchange(exchange)?;
                self.clear_rekey_exchange()?;
                return Ok(response);
            }
        };
        let key = dh_shared_key(&peer, &*secret, &p, Some(self.chat_id())).and_then(|key| {
            let key = Zeroizing::new(key);
            let claimed = action["fields"]["key_fingerprint"]
                .as_i64()
                .ok_or_else(|| self.rekey_rejected("invalid key fingerprint"))?;
            verify_dh_fingerprint(self.chat_id(), &*key, claimed)?;
            Ok(key)
        });
        let key = match key {
            Ok(key) => key,
            Err(_) => {
                let response = self.abort_remote_exchange(exchange)?;
                self.clear_rekey_exchange()?;
                return Ok(response);
            }
        };
        // The CommitKey ciphertext belongs to the old key; switch only once it exists.
        let prepared = self.prepare_service_action(&json!({"id":COMMIT,"fields":{
            "exchange_id":exchange,"key_fingerprint":key_fingerprint(&*key)?
        }}))?;
        self.adopt_rekey_key(KeySlot(*key), false)?;
        Ok(vec![prepared])
    }

    fn handle_key_commit(
        &mut self,
        exchange: i64,
        action: &Value,
    ) -> Result<Vec<PreparedServiceMessage>, SecretChatError> {
        if self.rekey.exchange != Some(exchange) || self.rekey.role != Some(ExchangeRole::Accepted)
        {
            return Ok(Vec::new());
        }
        let key = Zeroizing::new(
            self.rekey
                .pending
                .as_ref()
                .ok_or_else(|| self.rekey_rejected("missing pending key"))?
                .0,
        );
        let claimed = action["fields"]["key_fingerprint"]
            .as_i64()
            .ok_or_else(|| self.rekey_rejected("invalid key fingerprint"))?;
        if verify_dh_fingerprint(self.chat_id(), &*key, claimed).is_err() {
            self.close();
            return Err(self.rekey_rejected("rekey commitment failed its fingerprint check"));
        }
        // Prepare B's confirmation under the pending key before publishing adoption.
        let prepared = self.prepare_service_with_key(&json!({"id":NOOP,"fields":{}}), &key)?;
        self.adopt_rekey_key(KeySlot(*key), true)?;
        self.rekey.messages = 1;
        self.rekey.outgoing = 1;
        self.retire_previous_key_if_settled();
        Ok(vec![prepared])
    }

    fn abort_remote_exchange(
        &mut self,
        exchange: i64,
    ) -> Result<Vec<PreparedServiceMessage>, SecretChatError> {
        self.prepare_service_action(&json!({"id":ABORT,"fields":{"exchange_id":exchange}}))
            .map(|reply| vec![reply])
    }

    fn clear_rekey_exchange(&mut self) -> Result<(), SecretChatError> {
        self.rekey.exchange = None;
        self.rekey.role = None;
        self.rekey.secret = None;
        self.rekey.pending = None;
        self.key_lifecycle = KeyLifecycleState::Active {
            fingerprint: self.active_fingerprint()?,
        };
        Ok(())
    }

    fn active_fingerprint(&self) -> Result<i64, SecretChatError> {
        key_fingerprint(
            &self
                .active_key
                .as_ref()
                .ok_or_else(|| self.rekey_rejected("the chat has no active key"))?
                .0,
        )
    }

    fn mark_rekeying(&mut self, exchange_id: i64) -> Result<(), SecretChatError> {
        self.key_lifecycle = KeyLifecycleState::Rekeying {
            fingerprint: self.active_fingerprint()?,
            exchange_id,
        };
        Ok(())
    }

    pub(crate) fn adopt_rekey_key(
        &mut self,
        key: KeySlot,
        confirmed: bool,
    ) -> Result<(), SecretChatError> {
        self.rekey.previous = self.active_key.take();
        self.adopt_key(&key.0)?;
        self.clear_rekey_exchange()?;
        self.rekey.confirmed = confirmed;
        self.rekey.started = Instant::now();
        self.rekey.messages = 0;
        self.rekey.outgoing = 0;
        Ok(())
    }

    pub(crate) fn retire_previous_key_if_settled(&mut self) {
        if self.rekey.confirmed && !self.gap_requested() {
            self.rekey.previous = None;
        }
    }

    fn required_dh(&mut self) -> Result<(i64, Vec<u8>), SecretChatError> {
        if let Some(config) = &self.rekey.dh {
            return Ok(config.clone());
        }
        self.close();
        Err(self.rekey_rejected("stored chat lacks its DH parameters and cannot rekey"))
    }

    fn require_rekey_ready(&self) -> Result<(), SecretChatError> {
        match self.state() {
            ChatState::Ready => Ok(()),
            ChatState::Closed => Err(SecretChatError::ChatClosed {
                chat_id: self.chat_id(),
                reason: "chat is closed".into(),
            }),
            state => Err(SecretChatError::ChatNotReady {
                chat_id: self.chat_id(),
                state: format!("{state:?}").to_lowercase(),
            }),
        }
    }

    fn rekey_rejected(&self, reason: &str) -> SecretChatError {
        SecretChatError::ParameterRejected {
            chat_id: Some(self.chat_id()),
            reason: reason.into(),
        }
    }
}

fn new_secret() -> Result<Zeroizing<[u8; 256]>, SecretChatError> {
    let mut secret = Zeroizing::new(generate_secret_bytes()?);
    secret[0] |= 128;
    Ok(secret)
}

fn checked_public(
    g: i64,
    secret: &[u8; 256],
    p: &[u8],
    chat_id: i64,
) -> Result<Vec<u8>, SecretChatError> {
    let decimal = dh_public_value(&g.to_be_bytes(), secret, p)?;
    let bytes = num_bigint::BigUint::parse_bytes(decimal.as_bytes(), 10)
        .ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id: Some(chat_id),
            reason: "invalid generated DH value".into(),
        })?
        .to_bytes_be();
    check_peer_value(&bytes, p, Some(chat_id))?;
    let mut padded = vec![0u8; 256];
    let offset = padded.len().checked_sub(bytes.len()).ok_or_else(|| {
        SecretChatError::ParameterRejected {
            chat_id: Some(chat_id),
            reason: "generated DH value exceeds 2048 bits".into(),
        }
    })?;
    padded[offset..].copy_from_slice(&bytes);
    Ok(padded)
}

fn peer_bytes(action: &Value, field: &str) -> Result<Vec<u8>, ()> {
    let encoded = action["fields"][field]["hex"].as_str().ok_or(())?;
    if encoded.len() > 512 {
        return Err(());
    }
    hex::decode(encoded).map_err(|_| ())
}

fn exchange_id() -> Result<i64, String> {
    let mut raw = [0; 8];
    getrandom::fill(&mut raw).map_err(|_| "RuntimeError: OS randomness unavailable")?;
    Ok(i64::from_le_bytes(raw))
}

/// Port of `collision` (`rust/adapters/rekey_adapter.py:12`).
pub fn collision(mine: i64, theirs: i64) -> Result<String, String> {
    use std::cmp::Ordering;
    Ok(match mine.cmp(&theirs) {
        Ordering::Greater => "abandon_theirs",
        Ordering::Less => "join_theirs",
        Ordering::Equal => "abort_both",
    }
    .into())
}

/// Port of `exchange_id_contract` (`rust/adapters/rekey_adapter.py:16`).
pub fn exchange_id_contract() -> Result<bool, String> {
    let _id = exchange_id()?;
    Ok(true)
}

/// Port of `decisions` (`rust/adapters/rekey_adapter.py:21`).
///
/// `role` defaults to `""` in the source.
pub fn decisions(
    messages: i64,
    age: i64,
    exchange: bool,
    old_key: bool,
    confirmed: bool,
    gap: bool,
    role: String,
) -> Result<serde_json::Value, String> {
    let due = !exchange && !old_key && messages > 0 && (messages > 100 || age > 7 * 24 * 60 * 60);
    let may_abort = exchange && !matches!(role.as_str(), "accepted" | "committed");
    Ok(
        json!({"due": due, "may_abort": may_abort, "keeps_previous": old_key && (!confirmed || gap)}),
    )
}

/// Port of `key_state` (`rust/adapters/rekey_adapter.py:36`).
pub fn key_state(
    command: String,
    tag: i64,
    slot: i64,
) -> Result<Option<serde_json::Value>, String> {
    #[derive(Default)]
    struct KeyState {
        active: u8,
        previous: Option<u8>,
        pending: Option<u8>,
        confirmed: bool,
        messages: usize,
        exchange: Option<i64>,
        rekeying: bool,
    }
    let mut state = KeyState {
        previous: Some(2),
        pending: Some(1),
        ..KeyState::default()
    };
    match command.as_str() {
        "select" => {
            let wanted = u8::try_from(slot.rem_euclid(4)).map_err(|_| "Invalid synthetic slot")?;
            let fingerprint = |tag| Sha1::digest([tag; 256]);
            let target = fingerprint(wanted);
            let selected = [Some(state.active), state.pending, state.previous]
                .into_iter()
                .flatten()
                .find(|&id| fingerprint(id)[12..] == target[12..]);
            return Ok(Some(json!({"selected": selected})));
        }
        "adopt" => {
            state.previous = Some(state.active);
            state.active =
                u8::try_from(tag.rem_euclid(256)).map_err(|_| "Invalid synthetic tag")?;
            state.pending = None;
            state.exchange = None;
            state.confirmed = false;
            state.messages = 0;
        }
        "clear" => {
            state.pending = None;
            state.exchange = None;
            state.rekeying = false;
        }
        "retire" => state.previous = None,
        _ => return Ok(None),
    }
    Ok(Some(
        json!({"has_previous": state.previous.is_some(), "has_pending": state.pending.is_some(), "confirmed": state.confirmed, "messages": state.messages, "state": if state.rekeying {"rekeying"} else {"ready"}, "exchange": state.exchange.is_some(), "active_tag": state.active}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_and_commit_guards_match() -> Result<(), String> {
        assert_eq!(collision(2, 2)?, "abort_both");
        assert_eq!(collision(3, 2)?, "abandon_theirs");
        assert_eq!(collision(1, 2)?, "join_theirs");
        let kept = decisions(101, 604801, true, true, true, true, "committed".into())?;
        assert_eq!(
            kept,
            serde_json::json!({"due":false,"may_abort":false,"keeps_previous":true})
        );
        assert!(exchange_id_contract()?);
        Ok(())
    }
}
