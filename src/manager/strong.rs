//! Application of the verified caller client's committed ingress journal.
use super::*;
use crate::{
    custody::{CustodyLimits, CustodyStorage},
    ingress::{DurableIngress, IngressProtocolStorage},
    storage::StoreMode,
    transport::GrammersTransport,
};
use grammers_tl_types as tl;
type Base<T> = SecretChatManager<CustodyStorage<IngressProtocolStorage>, T>;
struct Replay {
    completion: Arc<Shutdown>,
    task: Option<JoinHandle<()>>,
    id: Id,
}
struct StrongInner<T: Transport> {
    manager: Base<T>,
    ingress: Arc<DurableIngress>,
    stopping: AtomicBool,
    replay: Mutex<Option<Replay>>,
    shutdown: Mutex<Option<Arc<Shutdown>>>,
}
/// Strong custody requires the verified pre-ack hook on the original caller pool.
/// Raw post-dispatch updates only wake journal replay and are never accepted as receipts.
pub struct StrongSecretChatManager<T: Transport = GrammersTransport> {
    inner: Arc<StrongInner<T>>,
}
impl<T: Transport> Clone for StrongSecretChatManager<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
fn completion() -> Arc<Shutdown> {
    Arc::new(Shutdown {
        result: Mutex::new(None),
        notify: Notify::new(),
    })
}
fn complete(job: &Shutdown, result: Result<()>) {
    if let Ok(mut slot) = job.result.lock()
        && slot.is_none()
    {
        *slot = Some(result);
    }
    job.notify.notify_waiters();
}
fn done(job: &Shutdown) -> bool {
    job.result.lock().is_ok_and(|r| r.is_some())
}
impl StrongSecretChatManager<GrammersTransport> {
    /// Prove and use the prepared original client; no client/session conversion occurs.
    pub async fn new(
        client: grammers_client::Client,
        ingress: Arc<DurableIngress>,
        limits: CustodyLimits,
    ) -> Result<Self> {
        Self::new_with_options(client, ingress, limits, Options::default()).await
    }
    /// Configure finite admission and callback cleanup on the verified caller client.
    pub async fn new_with_options(
        client: grammers_client::Client,
        ingress: Arc<DurableIngress>,
        limits: CustodyLimits,
        options: Options,
    ) -> Result<Self> {
        limits.validate()?;
        let seconds = ingress.limits()?.operation_seconds;
        let transport =
            tokio::time::timeout(Duration::from_secs_f64(seconds), ingress.transport(client))
                .await
                .map_err(|_| Error::IngressUnavailable)??;
        Self::with_transport_and_options(transport, ingress, limits, options).await
    }
    /// Adopt a native transport already proven by this exact ingress coordinator.
    pub async fn with_transport(
        transport: GrammersTransport,
        ingress: Arc<DurableIngress>,
        limits: CustodyLimits,
    ) -> Result<Self> {
        Self::with_transport_and_options(transport, ingress, limits, Options::default()).await
    }
    /// Configure finite admission on an already proven native transport.
    pub async fn with_transport_and_options(
        transport: GrammersTransport,
        ingress: Arc<DurableIngress>,
        limits: CustodyLimits,
        options: Options,
    ) -> Result<Self> {
        limits.validate()?;
        let binding = ingress.verified_binding()?;
        let expected = binding.digest();
        let actual = tokio::time::timeout(
            Duration::from_secs_f64(ingress.limits()?.operation_seconds),
            transport.authorization_digest(),
        )
        .await
        .map_err(|_| Error::IngressUnavailable)??;
        if actual.as_deref() != Some(expected.as_str()) {
            return Err(Error::Authorization);
        }
        let store = CustodyStorage::new(ingress.protocol_storage()?, expected.clone(), limits)?;
        let manager = SecretChatManager::new(transport, store, expected, options)?;
        Self::from_owned_manager(manager, ingress)
    }
}
impl<T: Transport> StrongSecretChatManager<T> {
    fn from_owned_manager(manager: Base<T>, ingress: Arc<DurableIngress>) -> Result<Self> {
        let binding = ingress.verified_binding()?;
        if !manager.protocol(|p| {
            Ok(p.mode() == StoreMode::Ingress && p.authorization_digest() == binding.digest())
        })? {
            return Err(Error::Authorization);
        }
        Ok(Self {
            inner: Arc::new(StrongInner {
                manager,
                ingress,
                stopping: AtomicBool::new(false),
                replay: Mutex::new(None),
                shutdown: Mutex::new(None),
            }),
        })
    }
    #[cfg(test)]
    pub(crate) fn test_from_manager(
        manager: Base<T>,
        ingress: Arc<DurableIngress>,
    ) -> Result<Self> {
        Self::from_owned_manager(manager, ingress)
    }
    #[cfg(test)]
    pub(crate) fn test_install_chat(&self, chat: Chat) -> Result<()> {
        self.inner.manager.protocol(|p| p.insert_chat(chat))
    }
    #[cfg(test)]
    pub(crate) fn test_receive_protocol(
        &self,
        id: i32,
        bytes: &[u8],
    ) -> Result<crate::protocol::Effects> {
        self.inner
            .manager
            .protocol(|p| p.receive(id, bytes, None, 124))
    }
    #[cfg(test)]
    pub(crate) fn test_drain_deliveries(&self, id: i32) -> transport::BoxFuture<'_, Result<()>> {
        Box::pin(self.inner.manager.drain_deliveries(id))
    }
    fn ready(&self) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            return Err(Error::IngressUnavailable);
        }
        self.inner.ingress.verified_binding()?;
        Ok(())
    }
    /// Start ordinary recovery, then replay only committed ingress output.
    pub async fn start(&self) -> Result<()> {
        self.ready()?;
        self.inner.manager.start().await?;
        self.drain_ingress().await
    }
    /// Wake journal replay; the supplied Raw update itself grants no acceptance.
    pub async fn handle_update(&self, _update: &tl::enums::Update) -> Result<()> {
        self.drain_ingress().await
    }
    /// Share one owned, finite replay task across concurrent wakeups.
    pub async fn drain_ingress(&self) -> Result<()> {
        self.ready()?;
        let job = {
            let mut slot = self.inner.replay.lock().map_err(|_| Error::Storage)?;
            if let Some(replay) = slot.as_ref().filter(|replay| !done(&replay.completion)) {
                Arc::clone(&replay.completion)
            } else {
                let job = completion();
                let finished = Arc::clone(&job);
                let manager = self.clone();
                let seconds = self.inner.ingress.limits()?.operation_seconds;
                let task = tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        Duration::from_secs_f64(seconds),
                        manager.replay(),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(Error::IngressUnavailable),
                    };
                    if result.is_err() {
                        tracing::warn!("Committed ingress replay awaits recovery");
                    }
                    complete(&finished, result);
                });
                let id = task.id();
                *slot = Some(Replay {
                    completion: Arc::clone(&job),
                    task: Some(task),
                    id,
                });
                job
            }
        };
        job.wait().await
    }
    async fn replay(&self) -> Result<()> {
        self.ready()?;
        for batch in self.inner.ingress.pending_batches()? {
            let output = batch.updates()?;
            for (index, (update, _peers)) in output.0.into_iter().enumerate() {
                self.ready()?;
                let Some(row) = self
                    .inner
                    .ingress
                    .pending_batches()?
                    .into_iter()
                    .find(|row| row.id == batch.id)
                else {
                    break;
                };
                if index < row.cursor || row.applied.contains(&index) {
                    continue;
                }
                let id = match &update {
                    tl::enums::Update::Encryption(e) => Some(e.chat.id()),
                    tl::enums::Update::NewEncryptedMessage(e) => Some(match &e.message {
                        tl::enums::EncryptedMessage::Message(m) => m.chat_id,
                        tl::enums::EncryptedMessage::Service(m) => m.chat_id,
                    }),
                    _ => None,
                };
                let Some(id) = id else {
                    self.inner.ingress.acknowledge_nonsecret(batch.id, index)?;
                    continue;
                };
                if id <= 0 {
                    return Err(Error::IngressUnavailable);
                }
                if self.inner.manager.status(id).is_ok()
                    && self.inner.manager.pause_status(id)?.paused
                {
                    continue;
                }
                if matches!(&update,tl::enums::Update::Encryption(e)if matches!(&e.chat,tl::enums::EncryptedChat::Discarded(_)))
                    && self.inner.manager.status(id).is_err()
                {
                    let settled = self.inner.manager.protocol(|protocol| {
                        let slots = self
                            .inner
                            .manager
                            .inner
                            .creation_slots
                            .lock()
                            .map_err(|_| Error::Storage)?;
                        if self.inner.manager.inner.creating.load(Ordering::Acquire) > 0
                            || !slots.is_empty()
                            || !protocol.private_record_names("pending-create/").is_empty()
                        {
                            return Ok(false);
                        }
                        self.inner
                            .ingress
                            .acknowledge_unknown_discarded(batch.id, index, false)?;
                        Ok(true)
                    })?;
                    if !settled && let tl::enums::Update::Encryption(update) = update {
                        // A request RPC may still reveal this chat ID. Keep its terminal
                        // update unacknowledged, while allowing healthy later rows to run.
                        self.inner.manager.handle_encryption(update.chat).await?;
                    }
                    continue;
                }
                let result = self
                    .inner
                    .ingress
                    .apply(batch.id, index, async {
                        match update {
                            tl::enums::Update::Encryption(e) => {
                                self.inner.manager.handle_encryption(e.chat).await
                            }
                            tl::enums::Update::NewEncryptedMessage(e) => match e.message {
                                tl::enums::EncryptedMessage::Message(m) => {
                                    self.inner
                                        .manager
                                        .receive(
                                            m.chat_id,
                                            &m.bytes,
                                            Some(crate::Serializable::to_bytes(&m.file)),
                                            m.date,
                                        )
                                        .await
                                }
                                tl::enums::EncryptedMessage::Service(m) => {
                                    self.inner
                                        .manager
                                        .receive(m.chat_id, &m.bytes, None, m.date)
                                        .await
                                }
                            },
                            _ => Err(Error::IngressUnavailable),
                        }
                    })
                    .await?;
                let applied = self
                    .inner
                    .ingress
                    .pending_batches()?
                    .into_iter()
                    .find(|row| row.id == batch.id)
                    .is_none_or(|row| index < row.cursor || row.applied.contains(&index));
                if !applied
                    && !self
                        .inner
                        .manager
                        .pause_status(id)
                        .is_ok_and(|status| status.paused)
                {
                    return Err(result.err().unwrap_or(Error::IngressUnavailable));
                }
            }
            self.inner.ingress.prune_completed()?;
        }
        Ok(())
    }
    /// Fence ACKs, cancel and join replay, then release the protocol writer.
    pub async fn stop(&self) -> Result<()> {
        let caller = tokio::task::try_id();
        if caller.is_some_and(|id| self.inner.manager.inner.dispatch.is_draining(id)) {
            return Ok(());
        }
        self.inner.ingress.fence()?;
        self.inner.stopping.store(true, Ordering::Release);
        self.inner.manager.fence_runtime();
        if caller.is_some_and(|id| {
            self.inner.replay.lock().is_ok_and(|r| {
                r.as_ref()
                    .is_some_and(|r| r.id == id && !done(&r.completion))
            })
        }) {
            return Err(Error::IngressUnavailable);
        }
        if let Some(id) = caller {
            self.inner
                .manager
                .inner
                .stop_callers
                .lock()
                .map_err(|_| Error::Storage)?
                .insert(id);
        }
        let _caller = StrongStopCaller {
            set: &self.inner.manager.inner.stop_callers,
            id: caller,
        };
        let job = {
            let mut slot = self.inner.shutdown.lock().map_err(|_| Error::Storage)?;
            if let Some(job) = slot.as_ref().filter(|job| !done(job)) {
                Arc::clone(job)
            } else {
                let job = completion();
                let finished = Arc::clone(&job);
                let manager = self.clone();
                tokio::spawn(async move {
                    let result = manager.finish_stop().await;
                    if result.is_err() {
                        tracing::error!("Strong ingress shutdown failed");
                    }
                    complete(&finished, result);
                });
                *slot = Some(Arc::clone(&job));
                job
            }
        };
        job.wait().await
    }
    async fn finish_stop(&self) -> Result<()> {
        let replay = {
            let mut slot = self.inner.replay.lock().map_err(|_| Error::Storage)?;
            slot.as_mut().and_then(|replay| {
                replay
                    .task
                    .take()
                    .map(|task| (replay.id, Arc::clone(&replay.completion), task))
            })
        };
        if let Some((id, job, mut task)) = replay {
            task.abort();
            if tokio::time::timeout(
                Duration::from_secs_f64(self.inner.ingress.limits()?.operation_seconds),
                &mut task,
            )
            .await
            .is_err()
            {
                if let Ok(mut slot) = self.inner.replay.lock()
                    && let Some(replay) = slot.as_mut().filter(|r| r.id == id)
                {
                    replay.task = Some(task);
                }
                return Err(Error::IngressUnavailable);
            }
            complete(&job, Err(Error::IngressUnavailable));
        }
        self.inner.manager.stop().await
    }
}
struct StrongStopCaller<'a> {
    set: &'a Mutex<HashSet<Id>>,
    id: Option<Id>,
}
impl Drop for StrongStopCaller<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id
            && let Ok(mut set) = self.set.lock()
        {
            set.remove(&id);
        }
    }
}

include!("strong_actions.rs");
