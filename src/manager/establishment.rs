use super::*;
use crate::{
    Deserializable, Serializable,
    chat::{ChatState, DhRecord},
    dh::{DhConfig, SecretExponent, verify_fingerprint},
    storage::{SecretRecord, StoreMode},
};
use grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingCreate {
    peer: Vec<u8>,
    peer_user_id: i64,
    request_id: i32,
    secret: SecretExponent,
    dh: DhRecord,
}
struct CreationSlot<S: Storage + 'static, T: Transport> {
    owner: std::sync::Weak<Inner<S, T>>,
    token: u64,
}
impl<S: Storage + 'static, T: Transport> CreationSlot<S, T> {
    fn record(&self, name: &str) -> Result<()> {
        let owner = self.owner.upgrade().ok_or(Error::ManagerStopping)?;
        let mut slots = owner.creation_slots.lock().map_err(|_| Error::Storage)?;
        *slots.get_mut(&self.token).ok_or(Error::ManagerStopping)? = Some(name.to_owned());
        Ok(())
    }
    fn release(&self) {
        if let Some(owner) = self.owner.upgrade()
            && let Ok(mut slots) = owner.creation_slots.lock()
        {
            slots.remove(&self.token);
        }
    }
}
impl<S: Storage + 'static, T: Transport> Drop for CreationSlot<S, T> {
    fn drop(&mut self) {
        self.release();
    }
}
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    fn reserve_creation(&self) -> Result<Option<CreationSlot<S, T>>> {
        self.generation()?;
        let Some(policy) = self.inner.work.policy else {
            return Ok(None);
        };
        if matches!(
            self.protocol(|p| Ok(p.mode()))?,
            StoreMode::Bounded | StoreMode::Ingress
        ) && (policy.records < 4 || policy.bytes < 16384)
        {
            return Err(Error::Paused);
        }
        let token = self.inner.reservation.fetch_add(1, Ordering::AcqRel);
        if token == 0 {
            return Err(Error::Capacity);
        }
        self.protocol(|p| {
            let mut slots = self
                .inner
                .creation_slots
                .lock()
                .map_err(|_| Error::Storage)?;
            let pending = p
                .private_record_names("pending-create/")
                .into_iter()
                .filter(|name| !slots.values().any(|record| record.as_ref() == Some(name)))
                .count();
            if p.chat_records()
                .len()
                .saturating_add(pending)
                .saturating_add(slots.len())
                >= policy.chats
            {
                return Err(Error::Paused);
            }
            slots.insert(token, None);
            Ok(())
        })?;
        Ok(Some(CreationSlot {
            owner: Arc::downgrade(&self.inner),
            token,
        }))
    }
    /// Persist a DH secret and request identity before invoking requestEncryption.
    pub async fn create(&self, user: tl::enums::InputUser) -> Result<ChatSnapshot> {
        let slot = self.reserve_creation()?;
        self.with_payload_operation(None, runtime::WorkKind::Create, 4096, async {
            self.ensure_binding().await?;
            let generation = self.generation()?;
            let peer_user_id = match &user {
                tl::enums::InputUser::User(user) => user.user_id,
                _ => return Err(Error::Media),
            };
            if peer_user_id <= 0 {
                return Err(Error::MediaField {
                    field: "peer_user_id",
                });
            }
            self.protocol(|p| p.admit(None, crate::storage::Operation::Create, 0))?;
            let dh = self.inner.transport.dh_config().await?;
            self.check_generation(generation)?;
            let request_id = i32::from_le_bytes(crate::crypto::random_bytes()?) & i32::MAX;
            let pending = PendingCreate {
                peer: user.to_bytes(),
                peer_user_id,
                request_id,
                secret: SecretExponent::generate()?,
                dh: DhRecord {
                    g: dh.generator(),
                    prime: dh.prime_bytes(),
                },
            };
            let record = format!("pending-create/{request_id}");
            if let Some(slot) = &slot {
                slot.record(&record)?;
            }
            self.protocol(|protocol| {
                if protocol.private_record(&record).is_some() {
                    return Err(Error::ChatState);
                }
                protocol.set_private_record(
                    record.clone(),
                    Some(SecretRecord::new(
                        serde_json::to_vec(&pending).map_err(|_| Error::Storage)?,
                    )),
                )
            })?;
            self.perform_create(record, pending, generation, slot.as_ref())
                .await
        })
        .await
    }
    async fn perform_create(
        &self,
        record: String,
        pending: PendingCreate,
        generation: u64,
        slot: Option<&CreationSlot<S, T>>,
    ) -> Result<ChatSnapshot> {
        self.check_generation(generation)?;
        let creation_generation = self.inner.creation_generation.load(Ordering::Acquire);
        let dh = DhConfig::new(pending.dh.g, &pending.dh.prime)?;
        let peer =
            tl::enums::InputUser::from_bytes(&pending.peer).map_err(|_| Error::InvalidState)?;
        self.inner.creating.fetch_add(1, Ordering::AcqRel);
        let creating = CreationGuard(&self.inner.creating);
        let response = self
            .inner
            .transport
            .request(peer, pending.request_id, dh.public_value(&pending.secret))
            .await;
        drop(creating);
        let encrypted = response.map_err(|_| Error::Transport)?;
        self.check_generation(generation)?;
        if self.inner.creation_generation.load(Ordering::Acquire) != creation_generation {
            return Err(Error::ManagerStopping);
        }
        let (id, access_hash) = match &encrypted {
            tl::enums::EncryptedChat::Waiting(chat) => (chat.id, chat.access_hash),
            tl::enums::EncryptedChat::Chat(chat) => (chat.id, chat.access_hash),
            tl::enums::EncryptedChat::Discarded(chat) => {
                self.protocol(|protocol| protocol.set_private_record(record, None))?;
                return Err(Error::ChatClosed { chat_id: chat.id });
            }
            _ => return Err(Error::ChatState),
        };
        let mut chat = Chat::new(
            id,
            access_hash,
            pending.peer_user_id,
            true,
            ChatState::Requested,
        );
        chat.dh = Some(pending.dh);
        chat.handshake_secret = Some(pending.secret);
        let installed = self.protocol(|protocol| {
            protocol.transaction(|state| {
                if state.chats.contains_key(&id) {
                    return Err(Error::ChatState);
                }
                state.chats.insert(id, chat);
                state.private_records.remove(&record);
                Ok(())
            })
        });
        if let Err(error) = installed {
            if self.protocol(|p| Ok(p.mode()))? == StoreMode::Durable {
                let _ = self.inner.transport.discard(id, false).await;
            }
            return Err(error);
        }
        if let Some(slot) = slot {
            slot.release();
        }
        self.bump_epoch(id)?;
        let early = self
            .inner
            .early
            .lock()
            .map_err(|_| Error::Storage)?
            .remove(&id);
        if let Some(terminal @ tl::enums::EncryptedChat::Discarded(_)) = early {
            self.handle_encryption(terminal).await?;
        } else if matches!(encrypted, tl::enums::EncryptedChat::Chat(_)) {
            self.handle_encryption(encrypted).await?;
        } else if let Some(early) = early {
            self.handle_encryption(early).await?;
        }
        self.status(id)
    }
    pub(crate) async fn recover_creations(&self) -> Result<()> {
        let names =
            self.protocol(|protocol| Ok(protocol.private_record_names("pending-create/")))?;
        for name in names {
            self.protocol(|p| p.admit(None, crate::storage::Operation::Recovery, 0))?;
            let pending = self.protocol(|protocol| {
                let record = protocol.private_record(&name).ok_or(Error::InvalidState)?;
                serde_json::from_slice::<PendingCreate>(record.as_bytes())
                    .map_err(|_| Error::InvalidState)
            })?;
            let generation = self.generation()?;
            if self
                .with_payload_operation(
                    None,
                    runtime::WorkKind::Create,
                    4096,
                    self.perform_create(name, pending, generation, None),
                )
                .await
                .is_err()
            {
                tracing::warn!("An initial chat request awaits retry");
            }
        }
        Ok(())
    }
    /// Accept an incoming request using its persisted exponent on every retry.
    pub async fn accept(&self, id: i32) -> Result<ChatSnapshot> {
        self.with_payload_operation(Some(id), runtime::WorkKind::Accept, 4096, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            let _headroom = self.reserve_headroom(id, 4, 16384)?;
            self.ensure_binding().await?;
            let generation = self.generation()?;
            self.protocol(|p| p.admit(Some(id), crate::storage::Operation::Accept, 0))?;
            let chat = self.require_chat(id)?;
            if matches!(chat.state, ChatState::Ready | ChatState::Rekeying) {
                return Ok(chat.snapshot());
            }
            if chat.state != ChatState::Waiting {
                return Err(Error::ChatNotReady { chat_id: id });
            }
            if chat.peer_public.is_none() {
                return Err(Error::ChatNotReady { chat_id: id });
            }
            if chat.handshake_secret.is_none() {
                let secret = SecretExponent::generate()?;
                self.protocol(|protocol| {
                    protocol.mutate_chat(id, |chat| {
                        if chat.handshake_secret.is_none() {
                            chat.handshake_secret = Some(secret);
                        }
                        Ok(())
                    })
                })?;
            }
            let chat = self.require_chat(id)?;
            let record = chat.dh.as_ref().ok_or(Error::DhParameters)?;
            let dh = DhConfig::new(record.g, &record.prime)?;
            let secret = chat.handshake_secret.as_ref().ok_or(Error::ChatState)?;
            let key =
                dh.shared_key(chat.peer_public.as_deref().ok_or(Error::ChatState)?, secret)?;
            let response = self
                .inner
                .transport
                .accept(
                    id,
                    chat.access_hash,
                    dh.public_value(secret),
                    key.fingerprint(),
                )
                .await;
            self.check_generation(generation)?;
            match response {
                Ok(_) | Err(RpcFailure::AlreadyAccepted) => {}
                Err(RpcFailure::ChatEnded | RpcFailure::Declined) => {
                    self.close_local(id, "the request is no longer open on the server", false)?;
                    return Err(Error::ChatClosed { chat_id: id });
                }
                Err(_) => return Err(Error::Transport),
            }
            let effect = self.protocol(|protocol| protocol.establish_existing(id, key, now()))?;
            self.emit_ready(id)?;
            self.transmit(effect, generation).await?;
            self.status(id)
        })
        .await
    }
    pub(crate) fn emit_ready(&self, id: i32) -> Result<()> {
        let chat = self.status(id)?;
        self.emit(Event::ChatReady {
            chat_id: id,
            peer_user_id: chat.peer_user_id,
            key_fingerprint: chat.key_fingerprint.ok_or(Error::ChatState)?,
            key_hash: chat.key_hash,
        });
        Ok(())
    }
    /// Process a raw updateEncryption from the caller-owned update stream.
    pub async fn handle_encryption(&self, encrypted: tl::enums::EncryptedChat) -> Result<()> {
        let amount = match &encrypted {
            tl::enums::EncryptedChat::Requested(q) => q.g_a.len(),
            tl::enums::EncryptedChat::Chat(q) => q.g_a_or_b.len(),
            _ => 0,
        }
        .saturating_add(512);
        let new_request = matches!(&encrypted,tl::enums::EncryptedChat::Requested(request) if self.protocol(|p|Ok(p.chat_record(request.id).is_err()))?);
        self.with_payload_operation(
            if new_request {
                None
            } else {
                Some(encrypted.id())
            },
            if new_request {
                runtime::WorkKind::Accept
            } else {
                runtime::WorkKind::Receive
            },
            amount,
            async {
                self.ensure_binding().await?;
                let generation = self.generation()?;
                match encrypted {
                    tl::enums::EncryptedChat::Requested(request) => {
                        if self.protocol(|protocol| Ok(protocol.chat_record(request.id).is_ok()))? {
                            self.protocol(|p| p.acknowledge_encryption_update(request.id))?;
                            return Ok(());
                        }
                        if !self.inner.accepting_requests.load(Ordering::Acquire) {
                            return Err(Error::Paused);
                        }
                        let slot = self.reserve_creation()?;
                        let creation_generation =
                            self.inner.creation_generation.load(Ordering::Acquire);
                        self.protocol(|p| {
                            p.admit(None, crate::storage::Operation::Accept, request.g_a.len())
                        })?;
                        let dh = self.inner.transport.dh_config().await?;
                        self.check_generation(generation)?;
                        if self.inner.creation_generation.load(Ordering::Acquire)
                            != creation_generation
                        {
                            return Err(Error::Paused);
                        }
                        if let Err(error) = dh.check_peer(&request.g_a) {
                            if self.protocol(|p| Ok(p.mode()))? == StoreMode::Durable {
                                let _ = self.inner.transport.discard(request.id, false).await;
                            }
                            return Err(error);
                        }
                        let mut chat = Chat::new(
                            request.id,
                            request.access_hash,
                            request.admin_id,
                            false,
                            ChatState::Waiting,
                        );
                        chat.dh = Some(DhRecord {
                            g: dh.generator(),
                            prime: dh.prime_bytes(),
                        });
                        chat.peer_public = Some(request.g_a);
                        self.protocol(|protocol| protocol.insert_chat(chat))?;
                        if let Some(slot) = slot {
                            slot.release();
                        }
                        self.bump_epoch(request.id)?;
                        self.emit(Event::ChatRequested {
                            chat_id: request.id,
                            peer_user_id: request.admin_id,
                        });
                    }
                    tl::enums::EncryptedChat::Chat(ready) => {
                        let chat = match self.require_chat(ready.id) {
                            Ok(chat) => chat,
                            Err(_) => {
                                if self.inner.creating.load(Ordering::Acquire) > 0 {
                                    self.remember_early(tl::enums::EncryptedChat::Chat(ready))?;
                                } else {
                                    self.discard_lost(ready.id).await;
                                }
                                return Ok(());
                            }
                        };
                        if chat.state == ChatState::Closed || chat.key.is_some() {
                            self.protocol(|p| p.acknowledge_encryption_update(ready.id))?;
                            return Ok(());
                        }
                        let Some(secret) = chat.handshake_secret.as_ref() else {
                            self.close_local(
                                ready.id,
                                "the request was accepted by another device",
                                false,
                            )?;
                            return Ok(());
                        };
                        let config = chat.dh.as_ref().ok_or(Error::DhParameters)?;
                        let dh = DhConfig::new(config.g, &config.prime)?;
                        let key = match dh.shared_key(&ready.g_a_or_b, secret).and_then(|key| {
                            verify_fingerprint(&key, ready.key_fingerprint)?;
                            Ok(key)
                        }) {
                            Ok(key) => key,
                            Err(error) => {
                                self.close(ready.id).await?;
                                return Err(error);
                            }
                        };
                        let effect = self.protocol(|protocol| {
                            protocol.establish_existing(ready.id, key, now())
                        })?;
                        self.emit_ready(ready.id)?;
                        self.transmit(effect, generation).await?;
                    }
                    tl::enums::EncryptedChat::Waiting(waiting) => {
                        if self.require_chat(waiting.id).is_err() {
                            if self.inner.creating.load(Ordering::Acquire) > 0 {
                                self.remember_early(tl::enums::EncryptedChat::Waiting(waiting))?;
                            } else {
                                self.discard_lost(waiting.id).await;
                            }
                        } else {
                            self.protocol(|p| p.acknowledge_encryption_update(waiting.id))?;
                        }
                    }
                    tl::enums::EncryptedChat::Discarded(discarded) => {
                        if let Ok(chat) = self.require_chat(discarded.id) {
                            if discarded.history_deleted {
                                self.close_local(discarded.id, "the peer deleted the chat", true)?;
                                if self.protocol(|p| Ok(p.mode()))? == StoreMode::Durable {
                                    self.remove_chat(discarded.id)?;
                                }
                            } else if !matches!(
                                chat.state,
                                ChatState::Closed | ChatState::Suspended
                            ) && !chat.paused
                            {
                                self.close_local(
                                    discarded.id,
                                    "the peer discarded the chat",
                                    false,
                                )?;
                            } else {
                                self.protocol(|p| p.acknowledge_encryption_update(discarded.id))?;
                            }
                        } else if self.inner.creating.load(Ordering::Acquire) > 0 {
                            self.remember_early(tl::enums::EncryptedChat::Discarded(discarded))?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            },
        )
        .await
    }
    fn remember_early(&self, encrypted: tl::enums::EncryptedChat) -> Result<()> {
        let id = encrypted.id();
        let mut early = self.inner.early.lock().map_err(|_| Error::Storage)?;
        if matches!(early.get(&id), Some(tl::enums::EncryptedChat::Discarded(_))) {
            return Ok(());
        }
        if early.contains_key(&id) || early.len() < 100 {
            early.insert(id, encrypted);
        }
        Ok(())
    }
    async fn discard_lost(&self, id: i32) {
        let mode = self.protocol(|p| Ok(p.mode()));
        if matches!(mode, Ok(StoreMode::Durable)) {
            let _ = self.inner.transport.discard(id, false).await;
        }
        self.emit(Event::DecryptFailed {
            chat_id: id,
            reason: "request lost; discarded",
        });
    }
}
struct CreationGuard<'a>(&'a AtomicUsize);
impl Drop for CreationGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
