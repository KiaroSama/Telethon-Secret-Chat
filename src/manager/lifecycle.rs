use super::*;
use crate::{chat::ChatState, storage::StoreMode};
use tracing::instrument::WithSubscriber;
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    /// Resume retained sends and mailbox deliveries and reannounce pending requests.
    pub async fn start(&self) -> Result<()> {
        if tokio::task::try_id().is_some_and(|id| self.inner.dispatch.is_draining(id)) {
            return Err(Error::ManagerStopping);
        }
        let _start = self.inner.start_lock.lock().await;
        if self.inner.stopping.load(Ordering::Acquire) {
            self.stop().await?;
        }
        if self.inner.running.load(Ordering::Acquire) {
            return Ok(());
        }
        self.protocol(|protocol| {
            if !protocol.is_running() {
                let _ = protocol.start()?;
            }
            Ok(())
        })?;
        let generation = self.inner.generation.load(Ordering::Acquire);
        self.ensure_binding().await?;
        if self.inner.generation.load(Ordering::Acquire) != generation {
            return Err(Error::ManagerStopping);
        }
        let mode = self.protocol(|protocol| Ok(protocol.mode()))?;
        let saved = self.protocol(|protocol| Ok(protocol.setting("auto_save_secret_chats")))?;
        let default = self
            .inner
            .options
            .auto_save_folder
            .clone()
            .or(self.protocol(|protocol| Ok(protocol.default_auto_save_folder()))?);
        let (autosave, initial) = if matches!(mode, StoreMode::Protected | StoreMode::Ingress) {
            (AutoSave::default(), None)
        } else {
            AutoSave::load(saved, default.as_deref())?
        };
        if let Some(setting) = initial {
            self.protocol(|protocol| {
                protocol.set_setting("auto_save_secret_chats".into(), Some(setting))
            })?;
        }
        *self.inner.autosave.lock().map_err(|_| Error::Storage)? = autosave;
        self.inner
            .outbound
            .lock()
            .map_err(|_| Error::Storage)?
            .clear();
        self.inner.wire.lock().map_err(|_| Error::Storage)?.clear();
        self.inner
            .delivered
            .lock()
            .map_err(|_| Error::Storage)?
            .clear();
        self.inner
            .cancellations
            .lock()
            .map_err(|_| Error::Storage)?
            .clear();
        self.inner
            .diagnosed
            .lock()
            .map_err(|_| Error::Storage)?
            .clear();
        *self
            .inner
            .creation_cancel
            .lock()
            .map_err(|_| Error::Storage)? = Arc::default();
        self.inner.stopping.store(false, Ordering::Release);
        self.inner.running.store(true, Ordering::Release);
        self.start_expiry_watch()?;
        let chats = self.protocol(|protocol| Ok(protocol.chat_records()))?;
        for chat in chats {
            if chat.paused || chat.state == ChatState::Suspended {
                continue;
            }
            if chat.state == ChatState::Requested && chat.handshake_secret.is_none() {
                self.close_failed_protocol(
                    chat.id,
                    "legacy pending handshake has no recoverable secret",
                )
                .await?;
                continue;
            }
            if chat.state == ChatState::Waiting {
                self.emit(Event::ChatRequested {
                    chat_id: chat.id,
                    peer_user_id: chat.peer_user_id,
                });
            } else if matches!(chat.state, ChatState::Ready | ChatState::Rekeying) {
                if self.drain_deliveries(chat.id).await.is_err() {
                    tracing::warn!(chat_id = chat.id, "durable work awaiting retry");
                }
                if self.retry_pending(chat.id).await.is_err() {
                    tracing::warn!(chat_id = chat.id, "durable work awaiting retry");
                }
            }
        }
        self.recover_creations().await?;
        self.resume_saved_downloads()?;
        Ok(())
    }
    /// Coordinate one cancellation-isolated shutdown; a later start retries a failed save.
    pub async fn stop(&self) -> Result<()> {
        let caller = tokio::task::try_id();
        if caller.is_some_and(|id| self.inner.dispatch.is_draining(id)) {
            return Ok(());
        }
        if let Some(id) = caller {
            self.inner
                .stop_callers
                .lock()
                .map_err(|_| Error::Storage)?
                .insert(id);
        }
        let _caller_guard = StopCallerGuard {
            set: &self.inner.stop_callers,
            id: caller,
        };
        let shutdown = {
            let mut slot = self.inner.shutdown.lock().map_err(|_| Error::Storage)?;
            let reusable = slot.as_ref().is_some_and(|shutdown| {
                shutdown.result.lock().is_ok_and(|result| result.is_none())
            });
            if reusable {
                slot.as_ref().cloned().ok_or(Error::Storage)?
            } else if !self.protocol(|p| Ok(p.is_running()))? {
                if let Some(id) = caller {
                    self.inner
                        .stop_callers
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .remove(&id);
                }
                return Ok(());
            } else {
                self.fence_runtime();
                let shutdown = Arc::new(Shutdown {
                    result: Mutex::new(None),
                    notify: Notify::new(),
                });
                *slot = Some(Arc::clone(&shutdown));
                let manager = self.clone();
                let completed = Arc::clone(&shutdown);
                tokio::spawn(
                    async move {
                        let result = manager.finish_stop().await;
                        if result.is_err() {
                            tracing::error!("Secret-chat shutdown failed");
                        }
                        if let Ok(mut value) = completed.result.lock() {
                            *value = Some(result);
                        }
                        completed.notify.notify_waiters();
                    }
                    .with_current_subscriber(),
                );
                shutdown
            }
        };
        let result = shutdown.wait().await;
        if let Some(id) = caller {
            self.inner
                .stop_callers
                .lock()
                .map_err(|_| Error::Storage)?
                .remove(&id);
        }
        result
    }
    async fn finish_stop(&self) -> Result<()> {
        self.stop_expiry_watch().await?;
        let downloads =
            std::mem::take(&mut *self.inner.downloads.lock().map_err(|_| Error::Storage)?);
        for task in &downloads {
            task.abort();
        }
        for task in downloads {
            let _ = task.await;
        }
        self.protocol(Protocol::stop)?;
        let exempt = self
            .inner
            .stop_callers
            .lock()
            .map_err(|_| Error::Storage)?
            .clone();
        self.inner
            .dispatch
            .drain(
                &exempt,
                self.inner
                    .work
                    .policy
                    .map(|p| Duration::from_secs_f64(p.cleanup_seconds))
                    .unwrap_or(self.inner.options.handler_cleanup_timeout),
            )
            .await;
        self.inner.early.lock().map_err(|_| Error::Storage)?.clear();
        self.inner.running.store(false, Ordering::Release);
        self.inner.stopping.store(true, Ordering::Release);
        self.inner
            .binding_generation
            .store(u64::MAX, Ordering::Release);
        Ok(())
    }
}

struct StopCallerGuard<'a> {
    set: &'a Mutex<HashSet<Id>>,
    id: Option<Id>,
}
impl Drop for StopCallerGuard<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id
            && let Ok(mut callers) = self.set.lock()
        {
            callers.remove(&id);
        }
    }
}
#[cfg(test)]
mod tests;
