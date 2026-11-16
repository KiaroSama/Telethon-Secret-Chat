//! Restored state must contain complete, coherent protocol obligations.
use super::*;
use crate::Serializable;
impl Chat {
    pub fn validate(&self) -> Result<()> {
        if self.id <= 0
            || self.peer_user_id <= 0
            || self.in_seq_no < 0
            || self.out_seq_no < 0
            || self.peer_in_seq_no < 0
            || self.peer_in_seq_no > self.out_seq_no
            || self.out_seq_no > i32::MAX / 2
            || self.in_seq_no > i32::MAX / 2
            || self.ttl < 0
            || self.capability_layer < 46
            || (self.wrapper_layer != 0 && self.wrapper_layer < 46)
            || self.wrapper_layer > self.capability_layer
            || self.gaps.len() > 1000
            || self
                .initial_key_hash
                .as_ref()
                .is_some_and(|h| h.len() != 36)
        {
            return Err(Error::InvalidState);
        }
        if (matches!(self.state, ChatState::Ready | ChatState::Rekeying) && self.key.is_none())
            || (self.state == ChatState::Rekeying && self.dh.is_none())
        {
            return Err(Error::InvalidState);
        }
        if self.state == ChatState::Closed
            && (self.key.is_some()
                || self.previous_key.is_some()
                || self.exchange.is_some()
                || self.handshake_secret.is_some()
                || !self.outbox.is_empty()
                || !self.gaps.is_empty()
                || !self.deliveries.is_empty()
                || !self.service_deliveries.is_empty())
        {
            return Err(Error::InvalidState);
        }
        for (seq, item) in &self.outbox {
            if *seq != item.seq_no
                || *seq < 0
                || *seq / 2 >= self.out_seq_no
                || *seq % 2 != i32::from(self.is_outbound)
                || item.frame.len() < 40
                || item.frame.len() > crate::crypto::MAX_FRAME_LENGTH
                || (item.frame.len() - 24) % 16 != 0
            {
                return Err(Error::InvalidState);
            }
        }
        for (seq, item) in &self.gaps {
            self.received_wrapper(*seq, item)?;
            if *seq <= self.in_seq_no {
                return Err(Error::InvalidState);
            }
        }
        for (seq, item) in &self.deliveries {
            let wrapper = self.received_wrapper(*seq, item)?;
            if *seq >= self.in_seq_no
                || matches!(
                    wrapper.message,
                    crate::enums::DecryptedMessage::Service(_)
                        | crate::enums::DecryptedMessage::ServiceLegacyaa48327d(_)
                )
            {
                return Err(Error::InvalidState);
            }
        }
        for (seq, row) in &self.service_deliveries {
            self.service_wrapper(*seq, row)?;
        }
        Ok(())
    }
    pub(crate) fn service_wrapper(
        &self,
        raw: i32,
        row: &ServiceDelivery,
    ) -> Result<crate::types::DecryptedMessageLayer> {
        let wrapper = self.received_wrapper(raw, &row.envelope)?;
        if raw < 0
            || raw >= self.in_seq_no
            || self.deliveries.contains_key(&raw)
            || row.envelope.file.as_ref().is_some_and(|file| {
                file != &grammers_tl_types::enums::EncryptedFile::Empty.to_bytes()
            })
            || !matches!(
                wrapper.message,
                crate::enums::DecryptedMessage::Service(_)
                    | crate::enums::DecryptedMessage::ServiceLegacyaa48327d(_)
            )
        {
            return Err(Error::InvalidState);
        }
        Ok(wrapper)
    }
    fn received_wrapper(
        &self,
        raw: i32,
        row: &Envelope,
    ) -> Result<crate::types::DecryptedMessageLayer> {
        let wrapper = crate::framing::unwrap(&row.body).map_err(|_| Error::InvalidState)?;
        if raw < 0
            || wrapper.out_seq_no < 0
            || wrapper.in_seq_no < 0
            || wrapper.out_seq_no / 2 != raw
            || wrapper.out_seq_no % 2 != i32::from(!self.is_outbound)
            || wrapper.in_seq_no % 2 != i32::from(self.is_outbound)
        {
            return Err(Error::InvalidState);
        }
        Ok(wrapper)
    }
}
