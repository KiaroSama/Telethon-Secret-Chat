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
        let mut admitted = self.inner.generation.load(Ordering::Acquire);
        if self.inner.stopping.load(Ordering::Acquire) {
            admitted = self.stop_barrier(Some(admitted)).await?;
        }
        let running = self.inner.running.load(Ordering::Acquire);
        if running && self.inner.startup_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        if !running {
            self.inner.transport.resume_io()?;
            self.protocol(|protocol| {
                if self.inner.generation.load(Ordering::Acquire) != admitted {
                    return Err(Error::ManagerStopping);
                }
                if !protocol.is_running() {
                    let _ = protocol.start()?;
                }
                Ok(())
            })?;
            self.ensure_binding().await?;
            let _publication = self
                .inner
                .runtime_state
                .lock()
                .map_err(|_| Error::Storage)?;
            if self.inner.generation.load(Ordering::Acquire) != admitted {
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
                .delivering
                .lock()
                .map_err(|_| Error::Storage)?
                .clear();
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
            self.inner.dispatch.resume()?;
            self.inner.stopping.store(false, Ordering::Release);
            self.inner.running.store(true, Ordering::Release);
            self.start_expiry_watch()?;
        }
        let startup_generation = self.generation()?;
        let chats = self.protocol(|protocol| Ok(protocol.chat_records()))?;
        for chat in chats {
            self.check_generation(startup_generation)?;
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
        self.check_generation(startup_generation)?;
        self.recover_creations().await?;
        {
            let _publication = self
                .inner
                .runtime_state
                .lock()
                .map_err(|_| Error::Storage)?;
            self.check_generation(startup_generation)?;
            self.resume_saved_downloads()?;
            self.inner.startup_complete.store(true, Ordering::Release);
        }
        Ok(())
    }
    /// Coordinate one cancellation-isolated shutdown; a later start retries a failed save.
    pub async fn stop(&self) -> Result<()> {
        self.stop_barrier(None).await.map(|_| ())
    }
    async fn stop_barrier(&self, expected: Option<u64>) -> Result<u64> {
        let caller = tokio::task::try_id();
        if caller.is_some_and(|id| self.inner.dispatch.is_draining(id)) {
            return Ok(self.inner.generation.load(Ordering::Acquire));
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
                let shutdown = slot.as_ref().cloned().ok_or(Error::Storage)?;
                if expected.is_some_and(|generation| generation != shutdown.generation) {
                    return Err(Error::ManagerStopping);
                }
                shutdown
            } else {
                let generation = match expected {
                    Some(expected) => self.fence_runtime_from(expected)?,
                    None => self.fence_runtime(),
                };
                let shutdown = Arc::new(Shutdown {
                    generation,
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
        result.map(|_| shutdown.generation)
    }
    async fn finish_stop(&self) -> Result<()> {
        let mut failure = self.stop_expiry_watch().await.err();
        let downloads =
            std::mem::take(&mut *self.inner.downloads.lock().map_err(|_| Error::Storage)?);
        for task in &downloads {
            task.abort();
        }
        for task in downloads {
            let _ = task.await;
        }
        let cleanup = self
            .inner
            .work
            .policy
            .map(|p| Duration::from_secs_f64(p.cleanup_seconds))
            .unwrap_or(self.inner.options.handler_cleanup_timeout);
        let io_result = tokio::time::timeout(cleanup, self.inner.transport.drain_io()).await;
        let io_finished = match io_result {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                failure.get_or_insert(error);
                false
            }
            Err(_) => {
                failure.get_or_insert(Error::ManagerStopping);
                false
            }
        };
        let exempt = self
            .inner
            .stop_callers
            .lock()
            .map_err(|_| Error::Storage)?
            .clone();
        let callbacks_finished = match self.inner.dispatch.drain(&exempt, cleanup).await {
            Ok(()) => true,
            Err(error) => {
                failure.get_or_insert(error);
                false
            }
        };
        // Both blocking I/O and returned callback children retain the protocol
        // writer until their actual owners finish. A later stop retries cleanup.
        if io_finished
            && callbacks_finished
            && let Err(error) = self.protocol(Protocol::stop)
        {
            failure.get_or_insert(error);
        }
        self.inner.early.lock().map_err(|_| Error::Storage)?.clear();
        self.inner.running.store(false, Ordering::Release);
        self.inner.stopping.store(true, Ordering::Release);
        self.inner
            .binding_generation
            .store(u64::MAX, Ordering::Release);
        failure.map_or(Ok(()), Err)
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
