use super::*;
use crate::{
    chat::ChatState, enums, framing, protocol::SendEffect, rekey, storage::StoreMode, types,
};
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    pub(crate) async fn transmit(&self, effect: SendEffect, generation: u64) -> Result<()> {
        self.with_operation(Some(effect.chat_id), runtime::WorkKind::Ordinary, async {
            let id = effect.chat_id;
            let random_id = effect.outgoing.random_id;
            self.check_generation(generation)?;
            let wire = Arc::clone(
                self.inner
                    .wire
                    .lock()
                    .map_err(|_| Error::Storage)?
                    .entry(id)
                    .or_default(),
            );
            let _wire = wire.lock().await;
            self.ensure_binding().await?;
            self.check_generation(generation)?;
            let chat = self.sendable(id)?;
            if effect.outgoing.frame.len() < 40
                || !(effect.outgoing.frame.len() - 24).is_multiple_of(16)
            {
                self.close_failed_protocol(id, "a resend request could not be satisfied")
                    .await?;
                return Err(Error::ResendUnavailable);
            }
            let Some(row) = chat.outbox.get(&effect.outgoing.seq_no) else {
                return Ok(());
            };
            if !row.pending
                || row.random_id != random_id
                || row.frame != effect.outgoing.frame
                || row.service != effect.outgoing.service
                || row.file != effect.outgoing.file
                || row.obligation_generation != effect.outgoing.obligation_generation
            {
                return Ok(());
            }
            let witness = self.witness(id)?;
            self.check_witness(id, witness)?;
            self.protocol(|p| p.note_attempt(id, effect.outgoing.seq_no))?;
            runtime::note_pending(Arc::as_ptr(&self.inner) as usize, id, random_id);
            let response = self.inner.transport.send(&effect).await;
            if self.check_witness(id, witness).is_err() {
                return Err(Error::SendPending {
                    chat_id: id,
                    random_id,
                    cause: crate::error::PendingCause::Cancelled,
                });
            }
            match response {
                Ok(sent) => {
                    let saved = self.protocol(|protocol| {
                        protocol
                            .confirm_sent_with_file(&effect, sent.file.map(transport::file_handle))
                    });
                    if saved.is_err() {
                        return Err(Error::SendPending {
                            chat_id: id,
                            random_id,
                            cause: crate::error::PendingCause::Storage,
                        });
                    }
                    Ok(())
                }
                Err(RpcFailure::ChatEnded | RpcFailure::Declined) => {
                    self.close_local(id, "the chat is no longer open on the server", false)?;
                    Err(Error::ChatClosed { chat_id: id })
                }
                Err(failure @ (RpcFailure::Permanent | RpcFailure::Rejected(_))) => {
                    if self.protocol(|p| Ok(p.mode()))? != StoreMode::Durable {
                        self.protocol(|p| p.terminate_local(id))?;
                        return Err(Error::Paused);
                    }
                    if !effect.outgoing.service {
                        self.rewrite_deleted(id, &HashSet::from([random_id]))?;
                        self.emit(Event::SendFailed {
                            chat_id: id,
                            random_id,
                            cause: match failure {
                                RpcFailure::Rejected(error) => error.name(),
                                _ => "permanent Telegram rejection",
                            },
                        });
                    } else {
                        let close = self.protocol(|p| {
                            p.mutate_chat(id, |chat| {
                                let Some(row) = chat.outbox.get_mut(&effect.outgoing.seq_no) else {
                                    return Ok(false);
                                };
                                row.failures = row.failures.saturating_add(1);
                                Ok(row.failures >= 2)
                            })
                        })?;
                        if close {
                            self.close_local(
                                id,
                                "a protocol message was rejected by Telegram",
                                false,
                            )?;
                            return Err(Error::ChatClosed { chat_id: id });
                        }
                    }
                    Err(Error::SendPending {
                        chat_id: id,
                        random_id,
                        cause: crate::error::PendingCause::Transport,
                    })
                }
                Err(failure) => {
                    if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
                        self.protocol(|p| p.terminate_local(id))?;
                        return Err(Error::Paused);
                    }
                    Err(Error::SendPending {
                        chat_id: id,
                        random_id,
                        cause: if failure == RpcFailure::Cancelled {
                            crate::error::PendingCause::Cancelled
                        } else {
                            crate::error::PendingCause::Transport
                        },
                    })
                }
            }
        })
        .await
    }
    /// Retry uncertain sends with their original random ID and exact stored bytes.
    pub async fn retry_pending(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Retry, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.retry_pending_locked(id).await
        })
        .await
    }
    pub(crate) async fn retry_pending_locked(&self, id: i32) -> Result<()> {
        let generation = self.generation()?;
        let effects = self.protocol(|protocol| protocol.retry_pending(id))?;
        let mut first = None;
        for effect in effects {
            self.check_generation(generation)?;
            match self.transmit(effect, generation).await {
                Err(error @ Error::SendPending { .. }) => {
                    if first.is_none() {
                        first = Some(error);
                    }
                }
                Err(error) => return Err(error),
                Ok(()) => {}
            }
        }
        first.map_or(Ok(()), Err)
    }
    pub(crate) async fn rekey_if_due(&self, id: i32, generation: u64) -> Result<()> {
        let chat = self.sendable(id)?;
        if chat.state == ChatState::Ready
            && rekey::should_rekey(&chat, now())
            && let Some(effect) = self.prepare_rekey_or_close(id).await?
        {
            self.transmit(effect, generation).await?;
        }
        Ok(())
    }
    /// Parse Markdown by default, then send through the retained durable outbox.
    pub async fn send_message(
        &self,
        id: i32,
        text: &str,
        entities: Option<Vec<grammers_tl_types::enums::MessageEntity>>,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.with_payload_operation(
            Some(id),
            runtime::WorkKind::Ordinary,
            text.len()
                .saturating_add(entities.as_ref().map_or(0, |v| {
                    v.iter()
                        .map(|e| grammers_tl_types::Serializable::to_bytes(e).len())
                        .sum::<usize>()
                }))
                .saturating_add(128),
            async {
                let generation = self.generation()?;
                let outbound = self.outbound(id)?;
                let _order = outbound.lock().await;
                let _headroom = self.reserve_headroom(
                    id,
                    4,
                    text.len().saturating_mul(8).saturating_add(16384),
                )?;
                self.check_generation(generation)?;
                let chat = self.sendable(id)?;
                let (text, entities) = if let Some(entities) = entities {
                    (text.to_owned(), entities)
                } else {
                    grammers_client::parsers::parse_markdown_message(text)
                };
                let entities = crate::entities::convert(
                    entities,
                    framing::outgoing_layer(chat.capability_layer),
                );
                self.retry_pending_locked(id).await?;
                self.rekey_if_due(id, generation).await?;
                let effect = self.protocol(|p| {
                    p.send_message(
                        id,
                        text,
                        if entities.is_empty() {
                            None
                        } else {
                            Some(entities)
                        },
                        reply_to,
                    )
                })?;
                let random_id = effect.outgoing.random_id;
                self.save_outgoing(id, &effect.outgoing.body);
                self.transmit(effect, generation).await?;
                Ok(random_id)
            },
        )
        .await
    }
    /// Start a new exchange explicitly; completion follows subsequent raw updates.
    pub async fn rekey(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Rekey, async {
            let generation = self.generation()?;
            let outbound = self.outbound(id)?;
            let _order = outbound.lock().await;
            self.check_generation(generation)?;
            self.sendable(id)?;
            self.retry_pending_locked(id).await?;
            if let Some(effect) = self.prepare_rekey_or_close(id).await? {
                self.transmit(effect, generation).await?;
            }
            Ok(())
        })
        .await
    }
    async fn prepare_rekey_or_close(&self, id: i32) -> Result<Option<SendEffect>> {
        match self.protocol(|p| p.rekey(id)) {
            Err(Error::DhParameters) => {
                self.close_failed_protocol(
                    id,
                    "stored chat lacks its DH parameters and cannot rekey",
                )
                .await?;
                Err(Error::ChatClosed { chat_id: id })
            }
            result => result,
        }
    }
    async fn action(&self, id: i32, action: enums::DecryptedMessageAction) -> Result<()> {
        let generation = self.generation()?;
        self.sendable(id)?;
        self.retry_pending_locked(id).await?;
        self.rekey_if_due(id, generation).await?;
        let effect = self.protocol(|p| p.send_action(id, action))?;
        self.save_outgoing(id, &effect.outgoing.body);
        self.transmit(effect, generation).await
    }
    /// Transmit a timer for future messages; the peer enforces timed destruction.
    pub async fn set_ttl(&self, id: i32, seconds: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Ordinary, async {
            if seconds < 0 {
                return Err(Error::Media);
            }
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.action(
                id,
                types::DecryptedMessageActionSetMessageTtl {
                    ttl_seconds: seconds,
                }
                .into(),
            )
            .await
        })
        .await
    }
    /// Tell the peer which random message IDs were read.
    pub async fn mark_read(&self, id: i32, random_ids: Vec<i64>) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Ordinary, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.action(
                id,
                types::DecryptedMessageActionReadMessages { random_ids }.into(),
            )
            .await
        })
        .await
    }
    /// Tell the peer which messages were included in a screenshot.
    pub async fn screenshot(&self, id: i32, random_ids: Vec<i64>) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Ordinary, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.action(
                id,
                types::DecryptedMessageActionScreenshotMessages { random_ids }.into(),
            )
            .await
        })
        .await
    }
    /// Send a typing or another supported activity indicator.
    pub async fn set_typing(
        &self,
        id: i32,
        action: Option<enums::SendMessageAction>,
    ) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Ordinary, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.action(
                id,
                types::DecryptedMessageActionTyping {
                    action: action.unwrap_or_else(|| types::SendMessageTypingAction {}.into()),
                }
                .into(),
            )
            .await
        })
        .await
    }
    fn rewrite_deleted(&self, id: i32, ids: &HashSet<i64>) -> Result<()> {
        self.protocol(|p| {
            p.mutate_chat(id, |chat| {
                crate::protocol::rewrite_retained_as_deletes(chat, &ids.iter().copied().collect())
            })
        })?;
        if let Some(history) = self
            .inner
            .history
            .lock()
            .map_err(|_| Error::Storage)?
            .get_mut(&id)
        {
            history.retain(|m| !ids.contains(&m.random_id));
        }
        Ok(())
    }
    /// Withdraw local retained content before retrying a network send or delete action.
    pub async fn delete_messages(
        &self,
        id: i32,
        random_ids: impl IntoIterator<Item = i64> + Send,
    ) -> Result<()> {
        let random_ids: Vec<i64> = random_ids.into_iter().collect();
        self.with_operation(Some(id), runtime::WorkKind::Persistence, async {
            self.content_policy_delete()?;
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.sendable(id)?;
            self.rewrite_deleted(id, &random_ids.iter().copied().collect())?;
            self.action(
                id,
                types::DecryptedMessageActionDeleteMessages { random_ids }.into(),
            )
            .await
        })
        .await
    }
    /// Withdraw all conversation content while retaining required protocol actions.
    pub async fn flush_history(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Persistence, async {
            self.content_policy_delete()?;
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            let chat = self.sendable(id)?;
            let ids = chat
                .outbox
                .values()
                .filter(|row| !row.service)
                .map(|row| row.random_id)
                .collect();
            self.rewrite_deleted(id, &ids)?;
            self.inner
                .history
                .lock()
                .map_err(|_| Error::Storage)?
                .remove(&id);
            self.action(id, enums::DecryptedMessageAction::FlushHistory)
                .await
        })
        .await
    }
    fn content_policy_delete(&self) -> Result<()> {
        if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
            return Err(Error::Paused);
        }
        Ok(())
    }
    fn archive_policy_delete(&self) -> Result<()> {
        if self.protocol(|p| Ok(p.mode()))? != StoreMode::Durable {
            return Err(Error::Paused);
        }
        Ok(())
    }
    pub(crate) fn close_local(
        &self,
        id: i32,
        reason: &'static str,
        history_deleted: bool,
    ) -> Result<()> {
        let typed = match reason {
            "closed by this application" | "deleted for both sides" => {
                crate::error::CloseReason::Application
            }
            "the peer discarded the chat" | "the peer deleted the chat" => {
                crate::error::CloseReason::PeerDiscarded
            }
            "rekey commitment failed its fingerprint check" => {
                crate::error::CloseReason::RekeyFingerprint
            }
            "stored chat lacks its DH parameters and cannot rekey" => {
                crate::error::CloseReason::MissingDh
            }
            _ => crate::error::CloseReason::Protocol,
        };
        self.close_local_with_reason(id, reason, history_deleted, typed)
    }
    fn close_local_with_reason(
        &self,
        id: i32,
        reason: &'static str,
        history_deleted: bool,
        typed: crate::error::CloseReason,
    ) -> Result<()> {
        let mode = self.protocol(|p| Ok(p.mode()))?;
        if mode != StoreMode::Durable {
            self.cancel_runtime(id);
        }
        let result = self.protocol(|p| p.close_with_reason(id, typed));
        if mode == StoreMode::Protected {
            self.emit_suspended(id);
            return result;
        }
        result?;
        self.inner
            .history
            .lock()
            .map_err(|_| Error::Storage)?
            .remove(&id);
        self.emit(Event::ChatClosed {
            chat_id: id,
            reason,
            history_deleted,
        });
        Ok(())
    }
    pub(crate) fn remove_chat(&self, id: i32) -> Result<()> {
        self.protocol(|p| p.forget_chat(id))?;
        self.bump_epoch(id)?;
        self.inner
            .history
            .lock()
            .map_err(|_| Error::Storage)?
            .remove(&id);
        Ok(())
    }
    /// End locally; non-durable modes retain required custody and do not discard remotely.
    pub async fn close(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Ordinary, async {
            self.generation()?;
            let chat = self.require_chat(id)?;
            if chat.state == ChatState::Closed {
                return Ok(());
            }
            let mode = self.protocol(|p| Ok(p.mode()))?;
            self.close_local(id, "closed by this application", false)?;
            if mode == StoreMode::Durable && self.inner.transport.discard(id, false).await.is_err()
            {
                tracing::warn!(chat_id = id, "Remote chat close awaits operator attention");
            }
            Ok(())
        })
        .await
    }
    /// Forget only a closed durable-mode tombstone, leaving saved copies untouched.
    pub async fn forget(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Persistence, async {
            self.archive_policy_delete()?;
            self.generation()?;
            if self.require_chat(id)?.state != ChatState::Closed {
                return Err(Error::ChatState);
            }
            self.remove_chat(id)
        })
        .await
    }
    /// End and remove this side's durable protocol state, preserving saved copies.
    pub async fn delete_secret_chat(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Persistence, async {
            self.archive_policy_delete()?;
            self.close(id).await?;
            self.remove_chat(id)
        })
        .await
    }
    /// Remove local protocol state and best-effort request deletion at the peer.
    pub async fn delete_secret_chat_both_sides(&self, id: i32) -> Result<bool> {
        self.with_operation(Some(id), runtime::WorkKind::Persistence, async {
            self.archive_policy_delete()?;
            self.generation()?;
            if self.require_chat(id)?.state != ChatState::Closed {
                self.close_local(id, "deleted for both sides", true)?;
            }
            let reached = self.inner.transport.discard(id, true).await.is_ok();
            self.remove_chat(id)?;
            Ok(reached)
        })
        .await
    }
}
