use super::*;
use crate::{Serializable, framing, protocol, storage::StoreMode};
use grammers_tl_types as tl;
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    /// Process an update without allowing a malformed frame to escape as a task error.
    pub async fn handle_update(&self, update: &tl::enums::Update) {
        let (id, result) = match update {
            tl::enums::Update::Encryption(update) => (
                update.chat.id(),
                self.handle_encryption(update.chat.clone()).await,
            ),
            tl::enums::Update::NewEncryptedMessage(update) => {
                let (id, bytes, file, date) = match &update.message {
                    tl::enums::EncryptedMessage::Message(message) => (
                        message.chat_id,
                        &message.bytes,
                        Some(message.file.to_bytes()),
                        message.date,
                    ),
                    tl::enums::EncryptedMessage::Service(message) => {
                        (message.chat_id, &message.bytes, None, message.date)
                    }
                };
                (id, self.receive(id, bytes, file, date).await)
            }
            _ => return,
        };
        if result.is_err() {
            self.emit(Event::DecryptFailed {
                chat_id: id,
                reason: "update could not be parsed or processed; durable work may need retry",
            });
        }
    }
    /// Authenticate, commit and deliver encrypted input through the caller's stream.
    pub async fn receive(
        &self,
        id: i32,
        bytes: &[u8],
        file: Option<Vec<u8>>,
        date: i32,
    ) -> Result<()> {
        self.with_payload_operation(
            Some(id),
            runtime::WorkKind::Receive,
            bytes
                .len()
                .saturating_add(file.as_ref().map_or(0, Vec::len))
                .saturating_add(256),
            async {
                self.ensure_binding().await?;
                let _headroom = match self.reserve_headroom(
                    id,
                    2,
                    bytes.len().saturating_mul(8).saturating_add(4096),
                ) {
                    Ok(held) => held,
                    Err(error) => {
                        self.runtime_refuse(
                            Some(id),
                            crate::storage::RuntimeRefusal::Capacity,
                            true,
                        );
                        return Err(error);
                    }
                };
                let generation = self.generation()?;
                let effects = match self.protocol(|p| p.receive(id, bytes, file, date)) {
                    Ok(effects) => effects,
                    Err(error) => {
                        if matches!(error, Error::Storage | Error::InvalidState) {
                            self.runtime_refuse(
                                Some(id),
                                crate::storage::RuntimeRefusal::Cancelled,
                                true,
                            );
                        }
                        if matches!(
                            error,
                            Error::Sequence
                                | Error::ResendUnavailable
                                | Error::InnerMessageRejected
                                | Error::Fingerprint
                        ) {
                            let reason = match &error {
                                Error::ResendUnavailable => {
                                    "a resend request could not be satisfied"
                                }
                                Error::Fingerprint => {
                                    "rekey commitment failed its fingerprint check"
                                }
                                _ => "received protocol invariants were violated",
                            };
                            self.close_failed_protocol(id, reason).await?;
                        }
                        return Err(error);
                    }
                };
                for event in effects.events {
                    if let protocol::Event::MessageAcknowledged {
                        chat_id,
                        random_ids,
                    } = event
                    {
                        let seq_no = self.require_chat(chat_id)?.peer_in_seq_no;
                        self.emit(Event::MessageAcknowledged {
                            chat_id,
                            seq_no,
                            random_ids,
                        });
                    }
                }
                self.drain_deliveries(id).await?;
                for effect in effects.sends {
                    self.check_generation(generation)?;
                    self.transmit(effect, generation).await?;
                }
                let chat = self.require_chat(id)?;
                if !chat.gaps.is_empty()
                    && chat.outbox.values().any(|row| row.pending && row.service)
                {
                    self.retry_pending(id).await?;
                }
                Ok(())
            },
        )
        .await
    }
    pub(crate) async fn drain_deliveries(&self, id: i32) -> Result<()> {
        let owner = Arc::new(());
        {
            let mut delivering = self.inner.delivering.lock().map_err(|_| Error::Storage)?;
            if let Some((_, requested)) = delivering.get_mut(&id) {
                *requested = true;
                return Ok(());
            }
            delivering.insert(id, (Arc::clone(&owner), false));
        }
        let _guard = DeliveryGuard {
            set: &self.inner.delivering,
            id,
            owner,
        };
        loop {
            let result = self.deliver_pending(id).await;
            let mut delivering = self.inner.delivering.lock().map_err(|_| Error::Storage)?;
            let Some((current, requested)) = delivering.get_mut(&id) else {
                return result;
            };
            if !Arc::ptr_eq(current, &_guard.owner) {
                return result;
            }
            if result.is_ok() && std::mem::take(requested) {
                drop(delivering);
                continue;
            }
            delivering.remove(&id);
            return result;
        }
    }
    pub(crate) async fn close_failed_protocol(&self, id: i32, reason: &'static str) -> Result<()> {
        let generation = self.generation()?;
        let mode = self.protocol(|p| Ok(p.mode()))?;
        self.close_local(id, reason, false)?;
        if mode == StoreMode::Durable {
            self.check_generation(generation)?;
            if self.inner.transport.discard(id, false).await.is_err() {
                tracing::warn!("Remote protocol close awaits operator attention");
            }
        }
        Ok(())
    }
    async fn deliver_pending(&self, id: i32) -> Result<()> {
        loop {
            let chat = self.require_chat(id)?;
            if chat.state == crate::chat::ChatState::Closed || chat.paused {
                return Ok(());
            }
            let (ordinary, services) = self
                .protocol(|p| Ok((p.pending_deliveries(id)?, p.pending_service_deliveries(id)?)))?;
            let seen = self
                .inner
                .delivered
                .lock()
                .map_err(|_| Error::Storage)?
                .clone();
            let pending_acknowledgements = self
                .inner
                .pending_acknowledgements
                .lock()
                .map_err(|_| Error::Storage)?
                .clone();
            let normal = ordinary
                .into_iter()
                .filter(|seq| {
                    !seen.contains(&(id, *seq))
                        || pending_acknowledgements.contains(&(id, *seq, false))
                })
                .min();
            let service = services.iter().map(|(seq, _)| *seq).min();
            let selected = match (normal, service) {
                (None, None) => return Ok(()),
                (Some(seq), None) => (seq, false),
                (None, Some(seq)) => (seq, true),
                (Some(n), Some(s)) => {
                    if n < s {
                        (n, false)
                    } else {
                        (s, true)
                    }
                }
            };
            let seq = selected.0;
            if pending_acknowledgements.contains(&(id, seq, selected.1)) {
                self.protocol(|p| {
                    if selected.1 {
                        p.acknowledge_service_delivery(id, seq)
                    } else {
                        p.acknowledge_delivery(id, seq)
                    }
                })?;
                self.inner
                    .pending_acknowledgements
                    .lock()
                    .map_err(|_| Error::Storage)?
                    .remove(&(id, seq, selected.1));
                if !selected.1 {
                    self.inner
                        .delivered
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .remove(&(id, seq));
                }
                continue;
            }
            if selected.1 {
                let generation = self.generation()?;
                let row = services
                    .into_iter()
                    .find(|(number, _)| *number == seq)
                    .ok_or(Error::Sequence)?
                    .1;
                let action = match framing::unwrap(&row.envelope.body)?.message {
                    crate::enums::DecryptedMessage::Service(service) => service.action,
                    crate::enums::DecryptedMessage::ServiceLegacyaa48327d(service) => {
                        service.action
                    }
                    _ => return Err(Error::Sequence),
                };
                let (event, effects) = match self.protocol(|p| p.apply_service_delivery(id, seq)) {
                    Ok(applied) => applied,
                    Err(error) => {
                        let reason = match (&action, &error) {
                            (
                                crate::enums::DecryptedMessageAction::RequestKey(_),
                                Error::PeerValue,
                            ) => Some("rekey request failed its public-value check"),
                            (
                                crate::enums::DecryptedMessageAction::CommitKey(_),
                                Error::Fingerprint,
                            ) => Some("rekey commitment failed its fingerprint check"),
                            (
                                crate::enums::DecryptedMessageAction::RequestKey(_)
                                | crate::enums::DecryptedMessageAction::AcceptKey(_)
                                | crate::enums::DecryptedMessageAction::CommitKey(_),
                                Error::DhParameters,
                            ) => Some("stored chat lacks its DH parameters and cannot rekey"),
                            _ => None,
                        };
                        if let Some(reason) = reason {
                            self.close_failed_protocol(id, reason).await?;
                        }
                        return Err(error);
                    }
                };
                for effect in effects.sends {
                    self.transmit(effect, generation).await?;
                }
                if event.applied {
                    match &event.action {
                        crate::enums::DecryptedMessageAction::DeleteMessages(delete) => {
                            if let Some(history) = self
                                .inner
                                .history
                                .lock()
                                .map_err(|_| Error::Storage)?
                                .get_mut(&id)
                            {
                                history.retain(|m| !delete.random_ids.contains(&m.random_id));
                            }
                        }
                        crate::enums::DecryptedMessageAction::FlushHistory => {
                            self.inner
                                .history
                                .lock()
                                .map_err(|_| Error::Storage)?
                                .remove(&id);
                        }
                        _ => {}
                    }
                }
                let action_bytes = event.action.to_bytes();
                let plain = crate::autosave::plain_object(&action_bytes, "DecryptedMessageAction")?;
                self.save_service(id, &plain, false);
                let mut action_name = plain
                    .get("_")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                if let Some(first) = action_name.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                if !self.emit(Event::ServiceActionReceived {
                    chat_id: id,
                    action_name,
                    action: event.action,
                    applied: event.applied,
                }) {
                    return Err(Error::Capacity);
                }
                self.inner
                    .pending_acknowledgements
                    .lock()
                    .map_err(|_| Error::Storage)?
                    .insert((id, seq, true));
                if self.protocol(|p| {
                    Ok(p.pending_service_deliveries(id)?
                        .iter()
                        .any(|(number, _)| *number == seq))
                })? {
                    self.protocol(|p| p.acknowledge_service_delivery(id, seq))?;
                }
                self.inner
                    .pending_acknowledgements
                    .lock()
                    .map_err(|_| Error::Storage)?
                    .remove(&(id, seq, true));
            } else {
                let Some(envelope) = self.protocol(|p| p.delivery(id, seq))? else {
                    continue;
                };
                let wrapper = match framing::unwrap(&envelope.body) {
                    Ok(wrapper) => wrapper,
                    Err(error) => {
                        if self.protocol(|p| Ok(p.mode()))? != StoreMode::Durable {
                            self.runtime_refuse(
                                Some(id),
                                crate::storage::RuntimeRefusal::Cancelled,
                                true,
                            );
                            return Err(error);
                        }
                        self.protocol(|p| p.acknowledge_delivery(id, seq))?;
                        self.emit(Event::DecryptFailed{chat_id:id,reason:"durable delivery could not be parsed; invalid mailbox row dropped"});
                        continue;
                    }
                };
                if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected
                    && !self.inner.dispatch.has_handlers("MessageReceived")
                {
                    self.runtime_refuse(Some(id), crate::storage::RuntimeRefusal::Callback, false);
                    return Err(Error::Paused);
                }
                if let Some(event) = events::message_event(id, wrapper, envelope.file)? {
                    self.inner
                        .delivered
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .insert((id, seq));
                    {
                        let mut histories =
                            self.inner.history.lock().map_err(|_| Error::Storage)?;
                        let history = histories.entry(id).or_default();
                        if !history
                            .iter()
                            .any(|prior| prior.random_id == event.random_id)
                            && self.inner.options.history_limit > 0
                        {
                            while history.len() >= self.inner.options.history_limit {
                                history.pop_front();
                            }
                            history.push_back(event.clone());
                        }
                    }
                    self.save_received(&event);
                    if !self.emit(Event::MessageReceived(Box::new(event))) {
                        self.inner
                            .delivered
                            .lock()
                            .map_err(|_| Error::Storage)?
                            .remove(&(id, seq));
                        return Err(Error::Capacity);
                    }
                }
                let mode = self.protocol(|p| Ok(p.mode()))?;
                if mode == StoreMode::Protected && self.require_chat(id)?.paused {
                    return Err(Error::Paused);
                }
                if matches!(mode, StoreMode::Durable | StoreMode::Protected) {
                    self.inner
                        .pending_acknowledgements
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .insert((id, seq, false));
                    self.protocol(|p| p.acknowledge_delivery(id, seq))?;
                    self.inner
                        .pending_acknowledgements
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .remove(&(id, seq, false));
                    self.inner
                        .delivered
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .remove(&(id, seq));
                }
            }
        }
    }
    /// Complete a bounded/ingress relay obligation after the consumer's durable work.
    pub fn acknowledge_delivery(&self, id: i32, wire_sequence: i32) -> Result<()> {
        self.generation()?;
        let chat = self.require_chat(id)?;
        if wire_sequence < 0 || wire_sequence % 2 != i32::from(!chat.is_outbound) {
            return Err(Error::Sequence);
        }
        let sequence = wire_sequence / 2;
        self.protocol(|p| p.acknowledge_delivery(id, sequence))?;
        self.inner
            .pending_acknowledgements
            .lock()
            .map_err(|_| Error::Storage)?
            .remove(&(id, sequence, false));
        self.inner
            .delivered
            .lock()
            .map_err(|_| Error::Storage)?
            .remove(&(id, sequence));
        Ok(())
    }
    /// Retained consumer obligations, including messages already observed by a callback.
    /// Sequence IDs are the original wire IDs and remain stable across reconstruction.
    pub fn pending_relay_deliveries(&self, id: i32) -> Result<Vec<crate::custody::RelayDelivery>> {
        self.generation()?;
        self.protocol(|p| {
            p.pending_deliveries(id)?
                .into_iter()
                .map(|sequence| {
                    let message = p.delivery(id, sequence)?.ok_or(Error::InvalidState)?;
                    let wrapper = framing::unwrap(&message.body)?;
                    Ok(crate::custody::RelayDelivery {
                        chat_id: id,
                        seq_no: wrapper.out_seq_no,
                        message,
                    })
                })
                .collect()
        })
    }
}

struct DeliveryGuard<'a> {
    set: &'a Mutex<BTreeMap<i32, (Arc<()>, bool)>>,
    id: i32,
    owner: Arc<()>,
}
impl Drop for DeliveryGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.lock()
            && set
                .get(&self.id)
                .is_some_and(|(owner, _)| Arc::ptr_eq(owner, &self.owner))
        {
            set.remove(&self.id);
        }
    }
}
#[cfg(test)]
mod tests;
