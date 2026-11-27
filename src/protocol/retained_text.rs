use zeroize::Zeroizing;

use super::{
    DECRYPTED_MESSAGE_73_ID, MAX_ENCRYPTED_FRAME_LEN, SecretChat, decrypt_frame, unwrap_frame,
};
use crate::errors::SecretChatError;

impl SecretChat {
    /// Recover a legacy text frame's TTL without receiving or changing the frame.
    ///
    /// The caller must still compare its legacy TTL-and-text identity digest. A
    /// retired key cannot be reconstructed from ciphertext or that digest, so
    /// unavailable-key retries must use the existing key-independent resend API.
    pub(crate) fn retained_outgoing_text_ttl(
        &self,
        frame: &[u8],
        out_seq_no: i64,
        random_id: i64,
        text: &str,
    ) -> Result<Option<i64>, SecretChatError> {
        let corrupt = || SecretChatError::StoreCorrupt {
            chat_id: Some(self.chat_id()),
            reason: "retained outgoing text frame failed verification".into(),
        };
        if frame.len() < 40
            || frame.len() > MAX_ENCRYPTED_FRAME_LEN
            || !(frame.len() - 24).is_multiple_of(16)
        {
            return Err(corrupt());
        }
        // Key selection only reads held key slots; decrypt with the outgoing
        // direction instead of applying any receive-side replay or key state.
        let (_, key) = self.select_receiving_key(frame).map_err(|_| {
            SecretChatError::ParameterRejected {
                chat_id: Some(self.chat_id()),
                reason: "retained text cannot be verified with the available shared keys; use retry_pending to resend its unchanged ciphertext".into(),
            }
        })?;
        let body =
            Zeroizing::new(decrypt_frame(&*key, frame, self.out_x()).map_err(|_| corrupt())?);
        // Reuse normal wrapper validation without advancing the conversation.
        unwrap_frame(&body, Some(self.chat_id())).map_err(|_| corrupt())?;
        let decoded = crate::ported::schema_secret_tl::decode_object(
            hex::encode(&*body),
            "DecryptedMessageLayer".into(),
            0,
        )
        .map_err(|_| corrupt())?;
        let fields = &decoded["object"]["fields"];
        let input = fields["in_seq_no"].as_i64().ok_or_else(corrupt)?;
        let output = fields["out_seq_no"].as_i64().ok_or_else(corrupt)?;
        if input < 0
            || output < 0
            || input % 2 != i64::from(!self.is_outbound())
            || output % 2 != i64::from(self.is_outbound())
            || input / 2 > self.in_seq_no()
        {
            return Err(corrupt());
        }
        let message = &fields["message"];
        if message["id"].as_u64() != Some(u64::from(DECRYPTED_MESSAGE_73_ID)) {
            return Ok(None);
        }
        let message = &message["fields"];
        let ttl = message["ttl"].as_i64().ok_or_else(corrupt)?;
        if !(0..=i64::from(i32::MAX)).contains(&ttl) {
            return Err(corrupt());
        }
        let expected_output = out_seq_no
            .checked_mul(2)
            .and_then(|wire| wire.checked_add(i64::from(self.is_outbound())))
            .filter(|&wire| wire >= 0 && i32::try_from(wire).is_ok());
        if expected_output != Some(output)
            || message["random_id"].as_i64() != Some(random_id)
            || message["message"].as_str() != Some(text)
        {
            return Ok(None);
        }
        Ok(Some(ttl))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ChatRole, KeySlot, encrypt_frame, wrap_frame};
    use serde_json::json;

    // Direction-dependent KDF slices must differ in this negative-control key.
    const CURRENT_KEY: [u8; 256] = {
        let mut key = [0; 256];
        let mut index = 0;
        let mut value = 0x51_u8;
        while index < key.len() {
            key[index] = value;
            value = value.wrapping_add(37);
            index += 1;
        }
        key
    };
    const PREVIOUS_KEY: [u8; 256] = [0x62; 256];
    const PENDING_KEY: [u8; 256] = [0x73; 256];
    const RANDOM_ID: i64 = 201;
    const SEQUENCE: i64 = 3;
    const TEXT: &str = "retained legacy text";

    fn chat(role: ChatRole) -> SecretChat {
        let mut chat = SecretChat::new(7, 8, 9, "retained-text-audit", role);
        chat.adopt_key(&CURRENT_KEY).unwrap();
        chat.set_out_seq_no(SEQUENCE + 1);
        chat
    }

    fn frame(chat: &SecretChat, key: &[u8; 256], ttl: i64, random_id: i64, text: &str) -> Vec<u8> {
        let body = wrap_frame(
            &json!({"random_id": random_id, "ttl": ttl, "message": text}),
            73,
            i64::from(!chat.is_outbound()),
            SEQUENCE * 2 + i64::from(chat.is_outbound()),
        )
        .unwrap();
        encrypt_frame(key, &body, chat.out_x()).unwrap()
    }

    #[test]
    fn legacy_text_uses_current_previous_and_pending_keys_without_mutation() {
        for role in [ChatRole::Initiator, ChatRole::Acceptor] {
            let mut chat = chat(role);
            chat.rekey.previous = Some(KeySlot(PREVIOUS_KEY));
            chat.rekey.pending = Some(KeySlot(PENDING_KEY));
            for key in [CURRENT_KEY, PREVIOUS_KEY, PENDING_KEY] {
                let frame = frame(&chat, &key, 17, RANDOM_ID, TEXT);
                let before = serde_json::to_value(chat.export_runtime()).unwrap();
                assert_eq!(
                    chat.retained_outgoing_text_ttl(&frame, SEQUENCE, RANDOM_ID, TEXT)
                        .unwrap(),
                    Some(17)
                );
                // Avoid printing even synthetic key material when an assertion fails.
                assert!(
                    before == serde_json::to_value(chat.export_runtime()).unwrap(),
                    "legacy text verification must not mutate protocol state"
                );
            }
        }
    }

    #[test]
    fn authenticated_text_identifier_and_sequence_mismatches_are_refused() {
        let chat = chat(ChatRole::Acceptor);
        let frame = frame(&chat, &CURRENT_KEY, 11, RANDOM_ID, TEXT);
        for (sequence, random_id, text) in [
            (SEQUENCE + 1, RANDOM_ID, TEXT),
            (SEQUENCE, RANDOM_ID + 1, TEXT),
            (SEQUENCE, RANDOM_ID, "different text"),
        ] {
            assert_eq!(
                chat.retained_outgoing_text_ttl(&frame, sequence, random_id, text)
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn missing_legacy_key_has_fixed_resend_guidance_and_preserves_state() {
        let chat = chat(ChatRole::Initiator);
        let frame = frame(&chat, &PREVIOUS_KEY, 23, RANDOM_ID, TEXT);
        let before = serde_json::to_value(chat.export_runtime()).unwrap();
        let error = chat
            .retained_outgoing_text_ttl(&frame, SEQUENCE, RANDOM_ID, TEXT)
            .unwrap_err();
        assert!(matches!(
            error,
            SecretChatError::ParameterRejected { reason, .. }
                if reason.contains("retry_pending")
        ));
        assert!(before == serde_json::to_value(chat.export_runtime()).unwrap());
    }

    #[test]
    fn authentication_and_outgoing_direction_are_required() {
        let chat = chat(ChatRole::Acceptor);
        let body = wrap_frame(
            &json!({"random_id": RANDOM_ID, "ttl": 17, "message": TEXT}),
            73,
            1,
            SEQUENCE * 2,
        )
        .unwrap();
        let wrong_direction = encrypt_frame(&CURRENT_KEY, &body, chat.in_x()).unwrap();
        let mut altered = frame(&chat, &CURRENT_KEY, 17, RANDOM_ID, TEXT);
        altered[24] ^= 1;
        for frame in [wrong_direction, altered] {
            assert!(matches!(
                chat.retained_outgoing_text_ttl(&frame, SEQUENCE, RANDOM_ID, TEXT),
                Err(SecretChatError::StoreCorrupt { .. })
            ));
        }
    }

    #[test]
    fn authenticated_invalid_parity_acknowledgement_and_ttl_are_refused() {
        let chat = chat(ChatRole::Acceptor);
        for (input, output, ttl) in [(0, 6, 17), (1, 7, 17), (3, 6, 17), (1, 6, -1)] {
            let body = wrap_frame(
                &json!({"random_id": RANDOM_ID, "ttl": ttl, "message": TEXT}),
                73,
                input,
                output,
            )
            .unwrap();
            let frame = encrypt_frame(&CURRENT_KEY, &body, chat.out_x()).unwrap();
            assert!(matches!(
                chat.retained_outgoing_text_ttl(&frame, SEQUENCE, RANDOM_ID, TEXT),
                Err(SecretChatError::StoreCorrupt { .. })
            ));
        }
    }

    #[test]
    fn retained_frame_bounds_are_checked_before_key_selection() {
        let chat = chat(ChatRole::Initiator);
        for size in [0, 39, 41, MAX_ENCRYPTED_FRAME_LEN + 1] {
            assert!(matches!(
                chat.retained_outgoing_text_ttl(&vec![0; size], SEQUENCE, RANDOM_ID, TEXT),
                Err(SecretChatError::StoreCorrupt { .. })
            ));
        }
    }

    #[test]
    fn valid_ttl_and_text_boundaries_preserve_legacy_matching() {
        let chat = chat(ChatRole::Initiator);
        for (ttl, random_id, text) in [(0, 0, ""), (i64::from(i32::MAX), i64::MIN, TEXT)] {
            let frame = frame(&chat, &CURRENT_KEY, ttl, random_id, text);
            assert_eq!(
                chat.retained_outgoing_text_ttl(&frame, SEQUENCE, random_id, text)
                    .unwrap(),
                Some(ttl)
            );
        }
    }

    #[test]
    fn service_frames_are_not_text_retry_candidates() {
        let mut chat = chat(ChatRole::Acceptor);
        let service = chat
            .prepare_service_action(&json!({"id": 0xa82fdd63_u32, "fields": {}}))
            .unwrap();
        assert_eq!(
            chat.retained_outgoing_text_ttl(
                &service.frame,
                service.out_seq_no,
                service.random_id,
                TEXT,
            )
            .unwrap(),
            None
        );
    }
}
