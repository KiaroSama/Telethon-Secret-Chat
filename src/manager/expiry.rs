//! One owned timer for finite volatile queues and held payloads.
use super::*;
use crate::storage::StoreMode;
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    pub(crate) fn start_expiry_watch(&self) -> Result<()> {
        if self.protocol(|p| Ok(p.mode()))? != StoreMode::Protected {
            return Ok(());
        }
        let mut slot = self.inner.expiry.lock().map_err(|_| Error::Storage)?;
        if slot.as_ref().is_some_and(|task| !task.is_finished()) {
            return Ok(());
        }
        let owner = Arc::downgrade(&self.inner);
        let changed = Arc::clone(&self.inner.expiry_changed);
        *slot = Some(tokio::spawn(async move {
            loop {
                let notification = changed.notified();
                tokio::pin!(notification);
                notification.as_mut().enable();
                let Some(inner) = owner.upgrade() else {
                    return;
                };
                let manager = Self { inner };
                if manager.inner.stopping.load(Ordering::Acquire) {
                    return;
                }
                let due = match manager.protocol(|p| p.next_runtime_expiry_ms()) {
                    Ok(due) => due,
                    Err(_) => {
                        tracing::warn!("Volatile expiry scheduling failed");
                        return;
                    }
                };
                drop(manager);
                if let Some(due) = due {
                    let current =
                        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                            Ok(time) => u64::try_from(time.as_millis()).unwrap_or(u64::MAX),
                            Err(_) => {
                                tracing::warn!("Volatile expiry clock failed");
                                return;
                            }
                        };
                    tokio::select! {_=notification=>continue,_=tokio::time::sleep(Duration::from_millis(due.saturating_sub(current)))=>{}}
                } else {
                    notification.await;
                    continue;
                }
                let Some(inner) = owner.upgrade() else {
                    return;
                };
                let manager = Self { inner };
                if manager.inner.stopping.load(Ordering::Acquire) {
                    return;
                }
                let expiry = match manager.protocol(|p| p.expire_runtime()) {
                    Ok(expiry) => expiry,
                    Err(_) => {
                        tracing::warn!("Volatile expiry could not be committed");
                        return;
                    }
                };
                for id in expiry.chats {
                    manager.cancel_runtime(id);
                    manager.emit_suspended(id);
                }
                if expiry.unbound {
                    manager
                        .inner
                        .creation_generation
                        .fetch_add(1, Ordering::AcqRel);
                    if let Ok(mut token) = manager.inner.creation_cancel.lock() {
                        token.cancel();
                        *token = Arc::default();
                    }
                }
                if expiry.persistence_failed {
                    tracing::warn!(
                        "Volatile expiry persistence failed; participation remains refused"
                    );
                }
            }
        }));
        Ok(())
    }
    pub(crate) async fn stop_expiry_watch(&self) -> Result<()> {
        let task = self.inner.expiry.lock().map_err(|_| Error::Storage)?.take();
        if let Some(task) = task {
            task.abort();
            if task.await.is_err_and(|error| !error.is_cancelled()) {
                return Err(Error::Storage);
            }
        }
        Ok(())
    }
}
impl<S: Storage, T: Transport> Drop for Inner<S, T> {
    fn drop(&mut self) {
        if let Ok(task) = self.expiry.get_mut()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}
