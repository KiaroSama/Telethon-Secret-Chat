//! Recoverable handler failures and cancellation cleanup are owned and observed.
use super::{
    events::Event,
    runtime::{self, PayloadReservation, WorkBudget, WorkPermit},
    transport::BoxFuture,
};
use crate::error::{Error, Result};
use std::{
    collections::{BTreeMap, HashSet},
    future::Future,
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
mod admission;
mod ownership;
use ownership::{Joined, Registry, Waiter};
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
    monitor_id: Id,
    finished: Arc<AtomicBool>,
    child_done: Arc<AtomicBool>,
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
    admissions: Arc<admission::Admissions>,
    closing_exempt: Arc<Mutex<HashSet<Id>>>,
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
            admissions: Arc::default(),
            closing_exempt: Arc::default(),
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
        let Ok(_admission) = self.admissions.enter(stopping) else {
            return false;
        };
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
        let reserve_payload = self.payload.lock().ok().and_then(|payload| payload.clone());
        let mut dispatches = Vec::with_capacity(callbacks.len());
        for (label, callback) in callbacks {
            let copies = match &callback {
                Callback::Async { cleanup, .. } | Callback::Task { cleanup, .. }
                    if cleanup.is_some() =>
                {
                    2
                }
                _ => 1,
            };
            let payload = match &reserve_payload {
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
            let waiter = if matches!(&callback, Callback::Task { .. })
                || (!stopping && matches!(&callback, Callback::Async { .. }))
            {
                match Waiter::prepare() {
                    Ok(waiter) => Some(waiter),
                    Err(_) => return false,
                }
            } else {
                None
            };
            dispatches.push((label, callback, payload, permit, waiter));
        }
        for (label, callback, payload, permit, waiter) in dispatches {
            if self.admissions.closed() && !stopping {
                return false;
            }
            match callback {
                Callback::Sync(handler) => {
                    if handler(event.clone()).is_err() {
                        tracing::error!(handler = label, "Secret-chat handler failed");
                    }
                    drop(permit);
                }
                Callback::Async { run, cleanup } => {
                    if !stopping {
                        let Some(waiter) = waiter else {
                            return false;
                        };
                        let data = event.clone();
                        let identity = self.identity.load(Ordering::Acquire);
                        let future = run(data);
                        let task = tokio::spawn(
                            runtime::scope(identity, future).with_current_subscriber(),
                        );
                        self.own(
                            task,
                            waiter,
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
                Callback::Task { run, cleanup } => {
                    let Some(waiter) = waiter else {
                        return false;
                    };
                    self.own(
                        run(event.clone()),
                        waiter,
                        OwnedCallback {
                            label,
                            cleanup: cleanup.map(|f| (f, event.clone())),
                            stopping,
                            generation,
                            chat_id: event.chat_id(),
                            permit,
                            payload,
                        },
                    );
                }
            }
        }
        true
    }
    fn own(&self, task: JoinHandle<Result<()>>, waiter: Waiter, work: OwnedCallback) {
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
        let finished = Arc::new(AtomicBool::new(false));
        let child_done = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Registry {
            tasks: self.tasks.clone(),
            changed: self.changed.clone(),
            id,
            finished: finished.clone(),
        });
        let receiver = waiter.handoff(task, permit, payload, registry.clone(), child_done.clone());
        let mut child = Box::pin(async move { receiver.await.map_err(|_| Error::Storage) });
        let cancel = Arc::new(Notify::new());
        let cancellation = Arc::clone(&cancel);
        let draining = Arc::clone(&self.draining);
        let failure = self.failure.lock().ok().and_then(|f| f.clone());
        let protected = self.protected.load(Ordering::Acquire);
        let policy = self.budget.policy;
        let identity = self.identity.load(Ordering::Acquire);
        let (start, started) = oneshot::channel();
        let monitor_abort = abort.clone();
        let monitor = tokio::spawn(async move {
            let _registration=registry;
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
            let (joined, timed_out) = tokio::select! {result=&mut child=>(result,false),_=cancellation.notified()=>{monitor_abort.abort();(child.await,false)},_=deadline=>{monitor_abort.abort();(child.await,true)}};
            let Ok(Joined{result,permit,payload,registry})=joined else {return;};
            let _resources=(permit,payload,registry);
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
                let mut cleanup=Box::pin(runtime::scope(identity,async move {cleanup(event).await}));
                let cleanup=std::future::poll_fn(move|cx| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(||cleanup.as_mut().poll(cx)))
                        .unwrap_or(std::task::Poll::Ready(Err(Error::ChatState)))
                });
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

        }.with_current_subscriber());
        let monitor_id = monitor.id();
        if stopping && let Ok(mut draining) = self.draining.lock() {
            draining.insert(monitor_id);
        }
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|entry| !entry.finished.load(Ordering::Acquire));
            tasks.push(Entry {
                id,
                chat_id,
                abort: abort.clone(),
                cancel: Arc::clone(&cancel),
                monitor: Some(monitor),
                monitor_id,
                finished,
                child_done,
            });
            tasks.retain(|entry| !entry.finished.load(Ordering::Acquire));
        }
        if stopping
            || (self.admissions.closed()
                && !self
                    .closing_exempt
                    .lock()
                    .is_ok_and(|ids| ids.contains(&id)))
        {
            abort.abort();
            if let Ok(mut draining) = self.draining.lock() {
                draining.insert(monitor_id);
            }
            cancel.notify_one();
        }
        let _ = start.send(());
    }
    pub fn cancel_chat(&self, id: i32) {
        let caller = tokio::task::try_id();
        if let Ok(tasks) = self.tasks.lock() {
            for entry in tasks.iter().filter(|e| e.chat_id == id) {
                if caller != Some(entry.id) && caller != Some(entry.monitor_id) {
                    entry.cancel.notify_one();
                    entry.abort.abort();
                }
            }
        }
    }
    pub fn is_draining(&self, id: Id) -> bool {
        self.draining.lock().is_ok_and(|set| set.contains(&id))
    }
    pub fn fence(&self) {
        self.admissions.fence();
    }
    pub fn fence_except(&self, exempt: &HashSet<Id>) {
        *self
            .closing_exempt
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = exempt.clone();
        self.fence();
    }
    pub fn resume(&self) -> Result<()> {
        if self.active() != 0 {
            return Err(Error::ManagerStopping);
        }
        self.closing_exempt
            .lock()
            .map_err(|_| Error::Storage)?
            .clear();
        self.admissions.reopen()
    }
    pub async fn drain(&self, exempt: &HashSet<Id>, cleanup_timeout: Duration) -> Result<()> {
        self.fence_except(exempt);
        let deadline = tokio::time::Instant::now() + cleanup_timeout;
        let through = self.admissions.checkpoint()?;
        tokio::time::timeout_at(deadline, self.admissions.wait(through))
            .await
            .map_err(|_| Error::ManagerStopping)??;
        self.drain_selected(
            |entry| !exempt.contains(&entry.id) && !exempt.contains(&entry.monitor_id),
            deadline.saturating_duration_since(tokio::time::Instant::now()),
        )
        .await
    }
    async fn drain_selected(
        &self,
        select: impl Fn(&Entry) -> bool,
        cleanup_timeout: Duration,
    ) -> Result<()> {
        let (mut entries, mut unfinished) = (Vec::new(), false);
        {
            let mut tasks = self.tasks.lock().map_err(|_| Error::Storage)?;
            for entry in tasks.iter_mut().filter(|entry| select(entry)) {
                if let Ok(mut draining) = self.draining.lock() {
                    draining.insert(entry.monitor_id);
                }
                entry.cancel.notify_one();
                entry.abort.abort();
                if let Some(monitor) = entry.monitor.take() {
                    entries.push((entry.id, monitor, entry.child_done.clone()));
                } else if !entry.finished.load(Ordering::Acquire) {
                    unfinished = true;
                }
            }
        }
        let deadline = tokio::time::Instant::now() + cleanup_timeout;
        for (id, mut monitor, child_done) in entries {
            if tokio::time::timeout_at(deadline, &mut monitor)
                .await
                .is_err()
            {
                if child_done.load(Ordering::Acquire) {
                    monitor.abort();
                    let _ = monitor.await;
                    tracing::warn!("Secret-chat handler cleanup exceeded its deadline");
                } else {
                    unfinished = true;
                    if let Some(entry) = self
                        .tasks
                        .lock()
                        .map_err(|_| Error::Storage)?
                        .iter_mut()
                        .find(|entry| entry.id == id)
                    {
                        entry.monitor = Some(monitor);
                    }
                }
            } else if !child_done.load(Ordering::Acquire) {
                // Runtime loss may end the monitor while the independent owner
                // still waits for a non-cancellable blocking callback.
                unfinished = true;
            }
        }
        self.changed.notify_waiters();
        if unfinished {
            Err(Error::ManagerStopping)
        } else {
            Ok(())
        }
    }
    pub async fn settle(&self) -> Result<()> {
        let caller = tokio::task::try_id();
        let through = self.admissions.checkpoint()?;
        if let Some(policy) = self.budget.policy {
            tokio::time::timeout(
                Duration::from_secs_f64(policy.operation_seconds),
                self.admissions.wait(through),
            )
            .await
            .map_err(|_| Error::Paused)??;
        } else {
            self.admissions.wait(through).await?;
        }
        let pending: HashSet<_> = self
            .tasks
            .lock()
            .map_err(|_| Error::Storage)?
            .iter()
            .filter(|entry| caller != Some(entry.id) && caller != Some(entry.monitor_id))
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
                    let _ = self
                        .drain_selected(
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
                    .filter(|e| !e.finished.load(Ordering::Acquire))
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
