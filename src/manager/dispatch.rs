//! Recoverable handler failures and cancellation cleanup are owned and observed.
use super::{
    events::Event,
    runtime::{self, PayloadReservation, WorkBudget, WorkPermit},
    transport::BoxFuture,
};
use crate::error::{Error, Result};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, oneshot},
    task::{AbortHandle, Id, JoinHandle},
};
use tracing::instrument::WithSubscriber;
type SyncHandler = Arc<dyn Fn(Event) -> Result<()> + Send + Sync>;
type AsyncHandler = Arc<dyn Fn(Event) -> BoxFuture<'static, Result<()>> + Send + Sync>;
type TaskHandler = Arc<dyn Fn(Event) -> JoinHandle<Result<()>> + Send + Sync>;
type PayloadFactory = Arc<dyn Fn(i32, usize) -> Result<PayloadReservation> + Send + Sync>;
type FailureHandler = Arc<dyn Fn(i32, u64) + Send + Sync>;
#[derive(Clone)]
enum Callback {
    Sync(SyncHandler),
    Async {
        run: AsyncHandler,
        cleanup: Option<AsyncHandler>,
    },
    Task {
        run: TaskHandler,
        cleanup: Option<AsyncHandler>,
    },
}
struct Entry {
    id: Id,
    chat_id: i32,
    abort: AbortHandle,
    cancel: Arc<Notify>,
    monitor: Option<JoinHandle<()>>,
}
type HandlerRegistry = BTreeMap<String, Vec<(&'static str, Callback)>>;
struct OwnedCallback {
    label: &'static str,
    cleanup: Option<(AsyncHandler, Event)>,
    stopping: bool,
    generation: u64,
    chat_id: i32,
    permit: WorkPermit,
    payload: PayloadReservation,
}
#[derive(Clone)]
pub struct Dispatcher {
    handlers: Arc<Mutex<HandlerRegistry>>,
    tasks: Arc<Mutex<Vec<Entry>>>,
    draining: Arc<Mutex<HashSet<Id>>>,
    budget: Arc<WorkBudget>,
    identity: Arc<AtomicUsize>,
    protected: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<FailureHandler>>>,
    payload: Arc<Mutex<Option<PayloadFactory>>>,
    changed: Arc<Notify>,
}
fn name(value: &str) -> Result<&str> {
    let value = if value == "ChatClosed" {
        "ChatClosedEvent"
    } else {
        value
    };
    if matches!(
        value,
        "ChatRequested"
            | "ChatReady"
            | "ChatClosedEvent"
            | "MessageReceived"
            | "MessageAcknowledged"
            | "ServiceActionReceived"
            | "DecryptFailed"
            | "SendFailed"
    ) {
        Ok(value)
    } else {
        Err(Error::ChatState)
    }
}
impl Dispatcher {
    pub fn new(budget: Arc<WorkBudget>) -> Self {
        Self {
            handlers: Arc::default(),
            tasks: Arc::default(),
            draining: Arc::default(),
            budget,
            identity: Arc::default(),
            protected: Arc::default(),
            failure: Arc::default(),
            payload: Arc::default(),
            changed: Arc::default(),
        }
    }
    pub fn configure(
        &self,
        identity: usize,
        protected: bool,
        failure: impl Fn(i32, u64) + Send + Sync + 'static,
        payload: impl Fn(i32, usize) -> Result<PayloadReservation> + Send + Sync + 'static,
    ) -> Result<()> {
        self.identity.store(identity, Ordering::Release);
        self.protected.store(protected, Ordering::Release);
        *self.failure.lock().map_err(|_| Error::Storage)? = Some(Arc::new(failure));
        *self.payload.lock().map_err(|_| Error::Storage)? = Some(Arc::new(payload));
        Ok(())
    }
    fn register(&self, event: &str, label: &'static str, callback: Callback) -> Result<()> {
        if label.is_empty()
            || label.len() > 128
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(Error::ChatState);
        }
        let mut handlers = self.handlers.lock().map_err(|_| Error::ChatState)?;
        if self.protected.load(Ordering::Acquire) {
            if matches!(&callback, Callback::Sync(_) | Callback::Task { .. }) {
                return Err(Error::MediaField { field: "handler" });
            }
            if self
                .budget
                .policy
                .is_some_and(|p| handlers.values().map(Vec::len).sum::<usize>() >= p.tasks)
            {
                return Err(Error::Capacity);
            }
        }
        handlers
            .entry(name(event)?.to_owned())
            .or_default()
            .push((label, callback));
        Ok(())
    }
    pub fn has_handlers(&self, event: &str) -> bool {
        self.handlers
            .lock()
            .is_ok_and(|h| h.get(event).is_some_and(|v| !v.is_empty()))
    }
    pub fn on(
        &self,
        event: &str,
        label: &'static str,
        handler: impl Fn(Event) -> Result<()> + Send + Sync + 'static,
    ) -> Result<()> {
        self.register(event, label, Callback::Sync(Arc::new(handler)))
    }
    pub fn on_async<F, Fut>(&self, event: &str, label: &'static str, handler: F) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.register(
            event,
            label,
            Callback::Async {
                run: Arc::new(move |event| Box::pin(handler(event))),
                cleanup: None,
            },
        )
    }
    pub fn on_async_with_cleanup<F, Fut, C, Cut>(
        &self,
        event: &str,
        label: &'static str,
        handler: F,
        cleanup: C,
    ) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.register(
            event,
            label,
            Callback::Async {
                run: Arc::new(move |event| Box::pin(handler(event))),
                cleanup: Some(Arc::new(move |event| Box::pin(cleanup(event)))),
            },
        )
    }
    pub fn on_task(
        &self,
        event: &str,
        label: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        self.register(
            event,
            label,
            Callback::Task {
                run: Arc::new(handler),
                cleanup: None,
            },
        )
    }
    pub fn on_task_with_cleanup<C, Cut>(
        &self,
        event: &str,
        label: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
        cleanup: C,
    ) -> Result<()>
    where
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.register(
            event,
            label,
            Callback::Task {
                run: Arc::new(handler),
                cleanup: Some(Arc::new(move |event| Box::pin(cleanup(event)))),
            },
        )
    }
    pub fn emit(&self, event: Event, stopping: bool, generation: u64) -> bool {
        if stopping && self.protected.load(Ordering::Acquire) {
            return true;
        }
        let callbacks = self
            .handlers
            .lock()
            .map(|h| h.get(event.name()).cloned().unwrap_or_default())
            .unwrap_or_default();
        let Ok(mut permits) = self.budget.acquire(callbacks.len()) else {
            tracing::warn!("Secret-chat callback budget is exhausted");
            return false;
        };
        for (label, callback) in callbacks {
            let copies = match &callback {
                Callback::Async { cleanup, .. } | Callback::Task { cleanup, .. }
                    if cleanup.is_some() =>
                {
                    2
                }
                _ => 1,
            };
            let payload = match self.payload.lock().ok().and_then(|p| p.clone()) {
                Some(reserve) => {
                    match reserve(event.chat_id(), event.payload_size().saturating_mul(copies)) {
                        Ok(reservation) => reservation,
                        Err(_) => return false,
                    }
                }
                None => PayloadReservation::empty(),
            };
            let Some(permit) = permits.take_one() else {
                return false;
            };
            match callback {
                Callback::Sync(handler) => {
                    if handler(event.clone()).is_err() {
                        tracing::error!(handler = label, "Secret-chat handler failed");
                    }
                    drop(permit);
                }
                Callback::Async { run, cleanup } => {
                    if !stopping {
                        let data = event.clone();
                        let identity = self.identity.load(Ordering::Acquire);
                        let future = run(data);
                        let task = tokio::spawn(
                            runtime::scope(identity, future).with_current_subscriber(),
                        );
                        self.own(
                            task,
                            OwnedCallback {
                                label,
                                cleanup: cleanup.map(|f| (f, event.clone())),
                                stopping: false,
                                generation,
                                chat_id: event.chat_id(),
                                permit,
                                payload,
                            },
                        );
                    }
                }
                Callback::Task { run, cleanup } => self.own(
                    run(event.clone()),
                    OwnedCallback {
                        label,
                        cleanup: cleanup.map(|f| (f, event.clone())),
                        stopping,
                        generation,
                        chat_id: event.chat_id(),
                        permit,
                        payload,
                    },
                ),
            }
        }
        true
    }
    fn own(&self, mut task: JoinHandle<Result<()>>, work: OwnedCallback) {
        let OwnedCallback {
            label,
            cleanup,
            stopping,
            generation,
            chat_id,
            permit,
            payload,
        } = work;
        let id = task.id();
        let abort = task.abort_handle();
        if stopping {
            abort.abort();
        }
        let tasks = Arc::clone(&self.tasks);
        let cancel = Arc::new(Notify::new());
        let cancellation = Arc::clone(&cancel);
        let draining = Arc::clone(&self.draining);
        let failure = self.failure.lock().ok().and_then(|f| f.clone());
        let protected = self.protected.load(Ordering::Acquire);
        let policy = self.budget.policy;
        let identity = self.identity.load(Ordering::Acquire);
        let (start, started) = oneshot::channel();
        let changed = Arc::clone(&self.changed);
        let monitor = tokio::spawn(async move {
            let _permit = permit;
            let _payload = payload;
            let _marker = DrainMarker {
                set: draining,
                id: tokio::task::id(),
            };
            let _ = started.await;
            let deadline = async {
                if let Some(policy) = policy {
                    tokio::time::sleep(Duration::from_secs_f64(policy.operation_seconds)).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            let (result, timed_out) = tokio::select! {result=&mut task=>(result,false),_=cancellation.notified()=>{task.abort();(task.await,false)},_=deadline=>{task.abort();(task.await,true)}};
            let cancelled = result.as_ref().is_err_and(|e| e.is_cancelled());
            let failed = timed_out
                || result.as_ref().is_err_and(|e| !e.is_cancelled())
                || result.as_ref().is_ok_and(Result::is_err);
            if timed_out {
                tracing::warn!(handler = label, "Secret-chat callback deadline reached");
            }
            match result {
                Ok(Ok(())) => {}
                Ok(Err(_)) => tracing::error!(handler = label, "Secret-chat handler failed"),
                Err(error) if error.is_cancelled() => {}
                Err(_) => tracing::error!(handler = label, "Secret-chat handler task terminated"),
            }
            if failed
                && protected
                && let Some(failure) = &failure
            {
                failure(chat_id, generation);
            }
            if cancelled && let Some((cleanup, event)) = cleanup {
                let cleanup = runtime::scope(identity, async move { cleanup(event).await });
                let result = if let Some(policy) = policy {
                    tokio::time::timeout(Duration::from_secs_f64(policy.cleanup_seconds), cleanup)
                        .await
                        .map_err(|_| Error::Paused)
                        .and_then(|r| r)
                } else {
                    cleanup.await
                };
                if result.is_err() {
                    tracing::error!(handler = label, "Secret-chat handler cleanup failed");
                    if protected && let Some(failure) = &failure {
                        failure(chat_id, generation);
                    }
                }
            }
            if let Ok(mut tasks) = tasks.lock() {
                tasks.retain(|entry| entry.id != id);
            }
            changed.notify_waiters();
        }.with_current_subscriber());
        if stopping && let Ok(mut draining) = self.draining.lock() {
            draining.insert(monitor.id());
        }
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|entry| entry.monitor.as_ref().is_some_and(|m| !m.is_finished()));
            tasks.push(Entry {
                id,
                chat_id,
                abort,
                cancel: Arc::clone(&cancel),
                monitor: Some(monitor),
            });
        }
        if stopping {
            cancel.notify_one();
        }
        let _ = start.send(());
    }
    pub fn cancel_chat(&self, id: i32) {
        let caller = tokio::task::try_id();
        if let Ok(tasks) = self.tasks.lock() {
            for entry in tasks.iter().filter(|e| e.chat_id == id) {
                if caller != Some(entry.id)
                    && !entry
                        .monitor
                        .as_ref()
                        .is_some_and(|m| caller == Some(m.id()))
                {
                    entry.cancel.notify_one();
                    entry.abort.abort();
                }
            }
        }
    }
    pub fn is_draining(&self, id: Id) -> bool {
        self.draining.lock().is_ok_and(|set| set.contains(&id))
    }
    pub async fn drain(&self, exempt: &HashSet<Id>, cleanup_timeout: Duration) {
        self.drain_selected(
            |entry| {
                !exempt.contains(&entry.id)
                    && !entry
                        .monitor
                        .as_ref()
                        .is_some_and(|m| exempt.contains(&m.id()))
            },
            cleanup_timeout,
        )
        .await;
    }
    async fn drain_selected(&self, select: impl Fn(&Entry) -> bool, cleanup_timeout: Duration) {
        let entries = if let Ok(mut tasks) = self.tasks.lock() {
            let (mut drain, mut kept) = (Vec::new(), Vec::new());
            for entry in tasks.drain(..) {
                if select(&entry) {
                    drain.push(entry);
                } else {
                    kept.push(entry);
                }
            }
            *tasks = kept;
            drain
        } else {
            Vec::new()
        };
        if let Ok(mut draining) = self.draining.lock() {
            for entry in &entries {
                if let Some(monitor) = &entry.monitor {
                    draining.insert(monitor.id());
                }
            }
        }
        for entry in &entries {
            entry.cancel.notify_one();
            entry.abort.abort();
        }
        let deadline = tokio::time::Instant::now() + cleanup_timeout;
        for mut entry in entries {
            if let Some(mut monitor) = entry.monitor.take()
                && tokio::time::timeout_at(deadline, &mut monitor)
                    .await
                    .is_err()
            {
                monitor.abort();
                let _ = monitor.await;
                tracing::warn!("Secret-chat handler cleanup exceeded its deadline");
            }
        }
        self.changed.notify_waiters();
    }
    pub async fn settle(&self) -> Result<()> {
        let caller = tokio::task::try_id();
        let pending: HashSet<_> = self
            .tasks
            .lock()
            .map_err(|_| Error::Storage)?
            .iter()
            .filter(|entry| {
                caller != Some(entry.id)
                    && !entry
                        .monitor
                        .as_ref()
                        .is_some_and(|monitor| caller == Some(monitor.id()))
            })
            .map(|entry| entry.id)
            .collect();
        let wait = async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if !self
                    .tasks
                    .lock()
                    .map_err(|_| Error::Storage)?
                    .iter()
                    .any(|entry| pending.contains(&entry.id))
                {
                    return Ok(());
                }
                changed.await;
            }
        };
        if let Some(policy) = self.budget.policy {
            match tokio::time::timeout(Duration::from_secs_f64(policy.operation_seconds), wait)
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    self.drain_selected(
                        |entry| pending.contains(&entry.id),
                        Duration::from_secs_f64(policy.cleanup_seconds),
                    )
                    .await;
                    Err(Error::Paused)
                }
            }
        } else {
            wait.await
        }
    }
    pub fn active(&self) -> usize {
        self.tasks
            .lock()
            .map(|tasks| {
                tasks
                    .iter()
                    .filter(|e| e.monitor.as_ref().is_some_and(|m| !m.is_finished()))
                    .count()
            })
            .unwrap_or(0)
    }
}
struct DrainMarker {
    set: Arc<Mutex<HashSet<Id>>>,
    id: Id,
}
impl Drop for DrainMarker {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.lock() {
            set.remove(&self.id);
        }
    }
}
