//! Controlled synchronous seams for the managed driver and custody adapters.
use super::*;
impl<S: Storage> Protocol<S> {
    #[cfg(test)]
    pub(crate) fn test_replace_deliveries(
        &mut self,
        id: i32,
        deliveries: std::collections::BTreeMap<i32, Envelope>,
    ) -> Result<()> {
        let chat = self
            .state
            .chats
            .get_mut(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?;
        // This private fault fixture corrupts mailbox bytes after receipt; keep
        // the receive counter coherent with the otherwise valid delivered rows.
        if let Some(sequence) = deliveries.keys().next_back() {
            chat.in_seq_no = chat
                .in_seq_no
                .max(sequence.checked_add(1).ok_or(Error::Sequence)?);
        }
        chat.deliveries = deliveries;
        Ok(())
    }
    pub fn admit_budget(&mut self, id: i32, records: usize, bytes: usize) -> Result<()> {
        let result = self.storage.admit_budget(id, records, bytes);
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    pub fn next_runtime_expiry_ms(&self) -> Result<Option<u64>> {
        self.storage.next_runtime_expiry_ms()
    }
    pub fn expire_runtime(&mut self) -> Result<crate::storage::RuntimeExpiry> {
        let result = self.storage.expire_runtime();
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    pub fn reserve_external(&mut self, id: Option<i32>, token: u64, bytes: usize) -> Result<()> {
        let result = self.storage.reserve_external(id, token, bytes);
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    pub fn release_external(&mut self, token: u64) {
        self.storage.release_external(token);
    }
    pub fn runtime_policy(&self) -> Option<crate::storage::RuntimePolicy> {
        self.storage.runtime_policy()
    }
    pub fn runtime_refused(
        &mut self,
        id: i32,
        reason: crate::storage::RuntimeRefusal,
        lost: bool,
    ) -> Result<()> {
        let result = self.storage.runtime_refused(id, reason, lost);
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    /// Consume a verified duplicate/terminal encryption update without changing
    /// chat counters. In strong mode this commits its matching journal cursor.
    pub fn acknowledge_encryption_update(&mut self, id: i32) -> Result<()> {
        self.ingress_transaction(
            id,
            crate::ingress::application::Kind::Encryption,
            |_| Ok(()),
        )
    }
    pub fn apply_service_delivery(
        &mut self,
        id: i32,
        raw_seq: i32,
    ) -> Result<(ServiceEvent, Effects)> {
        self.transaction(|state| {
            let chat = state
                .chats
                .get_mut(&id)
                .ok_or(Error::UnknownChat { chat_id: id })?;
            let row = chat
                .service_deliveries
                .get(&raw_seq)
                .cloned()
                .ok_or(Error::Sequence)?;
            let wrapper = chat.service_wrapper(raw_seq, &row)?;
            let action = match wrapper.message {
                enums::DecryptedMessage::Service(service) => service.action,
                enums::DecryptedMessage::ServiceLegacyaa48327d(service) => service.action,
                _ => return Err(Error::Sequence),
            };
            let mut effects = Effects::default();
            let applied = match row.applied {
                Some(applied) => applied,
                None => apply_action(chat, &action, &mut effects)?,
            };
            if let Some(saved) = chat.service_deliveries.get_mut(&raw_seq) {
                saved.applied = Some(applied);
            }
            crate::rekey::retire_if_settled(chat);
            let event = ServiceEvent {
                chat_id: id,
                seq_no: wrapper.out_seq_no,
                action,
                applied,
            };
            Ok((event, effects))
        })
    }
    pub fn pending_service_deliveries(
        &self,
        id: i32,
    ) -> Result<Vec<(i32, crate::chat::ServiceDelivery)>> {
        Ok(self
            .state
            .chats
            .get(&id)
            .ok_or(Error::UnknownChat { chat_id: id })?
            .service_deliveries
            .iter()
            .map(|(seq, row)| (*seq, row.clone()))
            .collect())
    }
    pub fn acknowledge_service_delivery(&mut self, id: i32, raw_seq: i32) -> Result<()> {
        self.transaction(|state| {
            state
                .chats
                .get_mut(&id)
                .ok_or(Error::UnknownChat { chat_id: id })?
                .service_deliveries
                .remove(&raw_seq)
                .ok_or(Error::Sequence)?;
            Ok(())
        })
    }
    pub fn authorization_digest(&self) -> &str {
        &self.state.authorization
    }
    pub fn default_auto_save_folder(&self) -> Option<std::path::PathBuf> {
        self.storage.default_auto_save_folder()
    }
    pub fn admit(
        &mut self,
        id: Option<i32>,
        operation: crate::storage::Operation,
        bytes: usize,
    ) -> Result<()> {
        if !self.running {
            return Err(Error::NotRunning);
        }
        let result = self.storage.admit(id, operation, bytes);
        if let Some(state) = self.storage.committed_view() {
            self.state = state;
        }
        result
    }
    pub fn note_attempt(&mut self, id: i32, wire_sequence: i32) -> Result<()> {
        let result = self.storage.note_attempt(id, wire_sequence);
        if let Some(state) = self.storage.committed_view() {
            self.state = state;
        }
        result
    }
    pub fn mode(&self) -> crate::storage::StoreMode {
        self.state.mode
    }
    pub fn terminate_local(&mut self, id: i32) -> Result<()> {
        self.close(id)
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    /// A detached protected record. It contains keys and must not be logged.
    pub fn chat_record(&self, id: i32) -> Result<Chat> {
        self.state.chats.get(&id).cloned().ok_or(Error::ChatState)
    }
    pub fn chat_records(&self) -> Vec<Chat> {
        self.state.chats.values().cloned().collect()
    }
    pub fn setting(&self, name: &str) -> Option<serde_json::Value> {
        self.state.settings.get(name).cloned()
    }
    pub fn set_setting(&mut self, name: String, value: Option<serde_json::Value>) -> Result<()> {
        self.transaction(|state| {
            if let Some(value) = value {
                state.settings.insert(name, value);
            } else {
                state.settings.remove(&name);
            }
            Ok(())
        })
    }
    pub fn private_record(&self, name: &str) -> Option<crate::storage::SecretRecord> {
        self.state.private_records.get(name).cloned()
    }
    pub fn private_record_names(&self, prefix: &str) -> Vec<String> {
        self.state
            .private_records
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect()
    }
    pub fn set_private_record(
        &mut self,
        name: String,
        value: Option<crate::storage::SecretRecord>,
    ) -> Result<()> {
        self.transaction(|state| {
            if let Some(value) = value {
                state.private_records.insert(name, value);
            } else {
                state.private_records.remove(&name);
            }
            Ok(())
        })
    }
    pub fn insert_chat(&mut self, chat: Chat) -> Result<()> {
        self.ingress_transaction(
            chat.id,
            crate::ingress::application::Kind::Encryption,
            |state| {
                if state.chats.contains_key(&chat.id) {
                    return Err(Error::ChatState);
                }
                state.chats.insert(chat.id, chat);
                Ok(())
            },
        )
    }
    pub fn mutate_chat<T>(&mut self, id: i32, f: impl FnOnce(&mut Chat) -> Result<T>) -> Result<T> {
        self.transaction(|state| f(state.chats.get_mut(&id).ok_or(Error::ChatState)?))
    }
    pub fn forget_chat(&mut self, id: i32) -> Result<()> {
        self.transaction(|state| {
            state.chats.remove(&id).ok_or(Error::ChatState)?;
            Ok(())
        })
    }
    pub fn prepare_message(
        &mut self,
        id: i32,
        message: enums::DecryptedMessage,
        file: Option<Vec<u8>>,
    ) -> Result<SendEffect> {
        self.admit(
            Some(id),
            crate::storage::Operation::Send,
            message.to_bytes().len() + file.as_ref().map_or(0, Vec::len),
        )?;
        let service = matches!(
            message,
            enums::DecryptedMessage::Service(_) | enums::DecryptedMessage::ServiceLegacyaa48327d(_)
        );
        if service && file.is_some() {
            return Err(Error::Media);
        }
        self.transaction(|state| {
            prepare(
                state.chats.get_mut(&id).ok_or(Error::ChatState)?,
                message,
                service,
                file,
            )
        })
    }
    pub fn establish_existing(&mut self, id: i32, key: SharedKey, now: u64) -> Result<SendEffect> {
        self.ingress_transaction(id, crate::ingress::application::Kind::Encryption, |state| {
            let chat = state.chats.get_mut(&id).ok_or(Error::ChatState)?;
            if !matches!(chat.state, ChatState::Requested | ChatState::Waiting)
                || chat.key.is_some()
            {
                return Err(Error::ChatState);
            }
            let dh = chat.dh.as_ref().ok_or(Error::DhParameters)?;
            crate::dh::DhConfig::new(dh.g, &dh.prime)?;
            chat.adopt_key(key)?;
            chat.handshake_secret = None;
            chat.peer_public = None;
            chat.state = ChatState::Ready;
            chat.rekeyed_at = now;
            prepare_action(
                chat,
                types::DecryptedMessageActionNotifyLayer { layer: 144 }.into(),
            )
        })
    }
}
