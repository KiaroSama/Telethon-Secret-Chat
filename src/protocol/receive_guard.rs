use serde_json::Value;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{SHARED_KEY_LEN, SecretChat, key_fingerprint, verify_dh_fingerprint};
use crate::errors::SecretChatError;
use crate::ported::rekey::ExchangeRole;

impl SecretChat {
    pub(crate) fn export_key_for_internal_frame(&self) -> Option<[u8; SHARED_KEY_LEN]> {
        self.active_key.as_ref().map(|slot| slot.0)
    }

    pub(super) fn select_receiving_key(
        &self,
        frame: &[u8],
    ) -> Result<(usize, Zeroizing<[u8; SHARED_KEY_LEN]>), SecretChatError> {
        let named = frame
            .get(..8)
            .ok_or_else(|| SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id()),
                reason: "the frame is too short to hold a message".into(),
            })?;
        [
            self.active_key.as_ref(),
            self.rekey.pending.as_ref(),
            self.rekey.previous.as_ref(),
        ]
        .into_iter()
        .enumerate()
        .find_map(|(index, slot)| {
            let slot = slot?;
            let fingerprint = key_fingerprint(&slot.0).ok()?;
            bool::from(named.ct_eq(&fingerprint.to_le_bytes()))
                .then(|| (index, Zeroizing::new(slot.0)))
        })
        .ok_or_else(|| SecretChatError::MessageRejected {
            chat_id: Some(self.chat_id()),
            reason: "the key fingerprint does not name a key this chat holds".into(),
        })
    }

    /// Call only after ciphertext authentication, and before decoding the inner body.
    pub(super) fn authenticated_replay(
        &mut self,
        body: &[u8],
        fingerprint: i64,
    ) -> Result<bool, SecretChatError> {
        let Ok((input, output)) = crate::ported::schema_secret_tl::wrapper_counters(body) else {
            return Ok(false);
        };
        let invalid_counters = input < 0
            || output < 0
            || input % 2 != i64::from(self.is_outbound())
            || output % 2 != i64::from(!self.is_outbound());
        let sequence = output / 2;
        let pending_key = self.pending_frame_keys.get(&sequence);
        let conflicting_keys = sequence >= self.in_seq_no
            && pending_key.is_some_and(|&pending| pending != fingerprint);
        if invalid_counters || conflicting_keys {
            self.close();
            return Err(SecretChatError::MessageRejected {
                chat_id: Some(self.chat_id()),
                reason: if invalid_counters {
                    "invalid authenticated sequence counter parity or sign"
                } else {
                    "one pending sequence was authenticated by conflicting shared keys"
                }
                .into(),
            });
        }
        Ok(sequence < self.in_seq_no || pending_key.is_some())
    }

    /// Check key provenance before delivering messages or exposing immediate actions.
    pub(super) fn received_key_cutover(
        &self,
        sequence: i64,
        fingerprint: i64,
        key_slot: usize,
        action: Option<&Value>,
    ) -> Result<Option<i64>, SecretChatError> {
        let rejected = |reason: &str| SecretChatError::MessageRejected {
            chat_id: Some(self.chat_id()),
            reason: reason.to_owned(),
        };
        let mut cutover = self.rekey.incoming_key_cutover;
        let mut tighten = |sequence| {
            cutover = Some(cutover.map_or(sequence, |current| current.min(sequence)));
        };
        if key_slot == 1 || (key_slot == 0 && self.rekey.previous.is_some()) {
            tighten(sequence);
        }

        if let Some(commit) = action.filter(|action| action["id"] == 0xec2e0b9b_u32) {
            let exchange = commit["fields"]["exchange_id"].as_i64();
            let pending_exchange = self
                .rekey
                .exchange
                .filter(|_| self.rekey.role == Some(ExchangeRole::Accepted));
            let (expected_key, preceding_slot) =
                if exchange.is_some() && exchange == pending_exchange {
                    (self.rekey.pending.as_ref(), 0)
                } else if exchange.is_some() && exchange == self.rekey.accepted_exchange {
                    (self.active_key.as_ref(), 2)
                } else {
                    (None, 0)
                };
            if let Some(key) = expected_key {
                if key_slot != preceding_slot {
                    return Err(rejected(
                        "the rekey commitment must use the preceding shared key",
                    ));
                }
                let claimed = commit["fields"]["key_fingerprint"]
                    .as_i64()
                    .ok_or_else(|| rejected("invalid rekey commitment fingerprint"))?;
                verify_dh_fingerprint(self.chat_id(), &key.0, claimed)
                    .map_err(|_| rejected("rekey commitment failed its fingerprint check"))?;
                // A delayed commitment can place the switch before the first new-key arrival.
                tighten(sequence + 1);
            }
        }

        let preceding_key = self
            .rekey
            .previous
            .as_ref()
            .or_else(|| self.rekey.pending.as_ref().and(self.active_key.as_ref()));
        if let (Some(cutover), Some(key)) = (cutover, preceding_key) {
            let preceding = key_fingerprint(&key.0)?;
            if (fingerprint == preceding && sequence >= cutover)
                || self
                    .pending_frame_keys
                    .iter()
                    .any(|(&sequence, &fingerprint)| {
                        fingerprint == preceding && sequence >= cutover
                    })
            {
                return Err(rejected(
                    "the previous shared key cannot authorize messages after the key switch",
                ));
            }
        }
        Ok(cutover)
    }
}
