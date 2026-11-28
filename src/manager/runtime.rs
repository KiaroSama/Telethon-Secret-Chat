use super::*;
use crate::storage::{RuntimePolicy, RuntimeRefusal, StoreMode};
use std::future::Future;
type PendingIdentity = Arc<Mutex<Option<(i32, i64)>>>;
struct Scope {
    identity: usize,
    pending: PendingIdentity,
    owner: u64,
}
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
tokio::task_local! {static SCOPE:Scope;}
pub(crate) async fn scope<F: Future>(identity: usize, future: F) -> F::Output {
    scope_pending(identity, Arc::default(), future).await
}
async fn scope_pending<F: Future>(
    identity: usize,
    pending: PendingIdentity,
    future: F,
) -> F::Output {
    SCOPE
        .scope(
            Scope {
                identity,
                pending,
                owner: NEXT_SCOPE.fetch_add(1, Ordering::AcqRel),
            },
            future,
        )
        .await
}
pub(super) fn nested(identity: usize) -> bool {
    SCOPE
        .try_with(|current| current.identity == identity)
        .unwrap_or(false)
}
pub(crate) fn note_pending(identity: usize, chat_id: i32, random_id: i64) {
    let _ = SCOPE.try_with(|scope| {
        if scope.identity == identity
            && let Ok(mut pending) = scope.pending.lock()
        {
            *pending = Some((chat_id, random_id));
        }
    });
}
pub(crate) struct WorkBudget {
    pub policy: Option<RuntimePolicy>,
    active: AtomicUsize,
    pub(crate) changed: Notify,
}
impl WorkBudget {
    pub fn new(policy: Option<RuntimePolicy>) -> Result<Arc<Self>> {
        if policy.is_some_and(|p| {
            !p.operation_seconds.is_finite()
                || p.operation_seconds <= 0.0
                || p.operation_seconds > 60.0
                || !p.cleanup_seconds.is_finite()
                || p.cleanup_seconds <= 0.0
                || p.cleanup_seconds > 30.0
                || p.tasks == 0
                || p.tasks > 64
        }) {
            return Err(Error::Capacity);
        }
        Ok(Arc::new(Self {
            policy,
            active: AtomicUsize::new(0),
            changed: Notify::new(),
        }))
    }
    pub fn acquire(self: &Arc<Self>, count: usize) -> Result<WorkPermit> {
        let mut current = self.active.load(Ordering::Acquire);
        loop {
            let next = current.checked_add(count).ok_or(Error::Capacity)?;
            if self.policy.is_some_and(|p| next > p.tasks) {
                return Err(Error::Capacity);
            }
            match self.active.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(WorkPermit {
                        budget: Arc::clone(self),
                        count,
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }
}
pub(crate) struct WorkPermit {
    budget: Arc<WorkBudget>,
    count: usize,
}
impl WorkPermit {
    pub fn take_one(&mut self) -> Option<Self> {
        if self.count == 0 {
            return None;
        }
        self.count -= 1;
        Some(Self {
            budget: Arc::clone(&self.budget),
            count: 1,
        })
    }
}
impl Drop for WorkPermit {
    fn drop(&mut self) {
        self.budget.active.fetch_sub(self.count, Ordering::AcqRel);
        self.budget.changed.notify_waiters();
    }
}
struct PayloadRelease(Mutex<Option<Box<dyn FnOnce() + Send>>>);
impl Drop for PayloadRelease {
    fn drop(&mut self) {
        if let Ok(mut callback) = self.0.lock()
            && let Some(release) = callback.take()
        {
            release();
        }
    }
}
pub(crate) struct PayloadReservation(Arc<PayloadRelease>);
impl PayloadReservation {
    pub fn empty() -> Self {
        Self(Arc::new(PayloadRelease(Mutex::new(None))))
    }
    fn new(release: impl FnOnce() + Send + 'static) -> Self {
        let resource = Arc::new(PayloadRelease(Mutex::new(Some(Box::new(release)))));
        crate::media::blocking::retain(&resource);
        Self(resource)
    }
}
#[derive(Default)]
pub(crate) struct Cancellation {
    cancelled: AtomicBool,
    notify: Notify,
}
impl Cancellation {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
    async fn wait(&self) {
        loop {
            let changed = self.notify.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkKind {
    Ordinary,
    Accept,
    Retry,
    Rekey,
    File,
    Receive,
    Create,
    Persistence,
    Resume,
}
#[derive(Default)]
pub(crate) struct Headroom(BTreeMap<u64, (usize, usize)>);
impl Headroom {
    fn maximum(&self) -> (usize, usize) {
        self.0.values().fold((0, 0), |(records, bytes), amount| {
            (records.max(amount.0), bytes.max(amount.1))
        })
    }
}
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    pub(crate) fn reserve_headroom(
        &self,
        id: i32,
        records: usize,
        bytes: usize,
    ) -> Result<PayloadReservation> {
        if !matches!(
            self.protocol(|p| Ok(p.mode()))?,
            StoreMode::Bounded | StoreMode::Ingress
        ) {
            return Ok(PayloadReservation::empty());
        }
        let identity = Arc::as_ptr(&self.inner) as usize;
        let owner = SCOPE
            .try_with(|scope| (scope.identity == identity).then_some(scope.owner))
            .ok()
            .flatten()
            .ok_or(Error::InvalidState)?;
        let lease = self.inner.reservation.fetch_add(1, Ordering::AcqRel);
        if lease == 0 {
            return Err(Error::Capacity);
        }
        self.protocol(|p| {
            let mut held = self.inner.headroom.lock().map_err(|_| Error::Storage)?;
            let key = (id, owner);
            let previous = held.get(&key).map(Headroom::maximum);
            let requested = (
                records.max(previous.map_or(0, |v| v.0)),
                bytes.max(previous.map_or(0, |v| v.1)),
            );
            let (other_records, other_bytes) = held
                .iter()
                .filter(|((chat, token), _)| *chat == id && *token != owner)
                .try_fold((0usize, 0usize), |(rs, bs), (_, amount)| {
                    Ok::<_, Error>((
                        rs.checked_add(amount.maximum().0).ok_or(Error::Capacity)?,
                        bs.checked_add(amount.maximum().1).ok_or(Error::Capacity)?,
                    ))
                })?;
            p.admit_budget(
                id,
                requested
                    .0
                    .checked_add(other_records)
                    .ok_or(Error::Capacity)?,
                requested
                    .1
                    .checked_add(other_bytes)
                    .ok_or(Error::Capacity)?,
            )?;
            held.entry(key)
                .or_default()
                .0
                .insert(lease, (records, bytes));
            Ok(())
        })?;
        let weak = Arc::downgrade(&self.inner);
        Ok(PayloadReservation::new(move || {
            if let Some(inner) = weak.upgrade()
                && let Ok(mut held) = inner.headroom.lock()
                && let Some(leases) = held.get_mut(&(id, owner))
            {
                leases.0.remove(&lease);
                if leases.0.is_empty() {
                    held.remove(&(id, owner));
                }
            }
        }))
    }
    pub(crate) fn runtime_refuse(&self, id: Option<i32>, reason: RuntimeRefusal, lost: bool) {
        if let Some(id) = id {
            self.cancel_runtime(id);
            if self
                .protocol(|p| p.runtime_refused(id, reason, lost))
                .is_err()
            {
                tracing::warn!("Secret-chat runtime refusal could not be committed");
            }
            if self
                .protocol(|p| Ok(p.mode()))
                .is_ok_and(|mode| mode == StoreMode::Protected)
            {
                self.emit_suspended(id);
            }
        }
    }
    pub(crate) fn emit_suspended(&self, id: i32) {
        if self
            .inner
            .diagnosed
            .lock()
            .is_ok_and(|mut diagnosed| diagnosed.insert(id))
        {
            self.emit(Event::DecryptFailed {
                chat_id: id,
                reason: "transient processing stopped; chat suspended",
            });
        }
    }
    pub(crate) fn cancel_runtime(&self, id: i32) {
        if self
            .protocol(|p| Ok(p.snapshot(id).is_ok()))
            .unwrap_or(false)
        {
            if let Ok(token) = self.cancellation(id) {
                token.cancel();
            }
            let _ = self.bump_epoch(id);
            self.inner.dispatch.cancel_chat(id);
        }
    }
    pub(crate) fn fence_runtime(&self) -> u64 {
        let _publication = self
            .inner
            .runtime_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.fence_runtime_locked()
    }
    pub(crate) fn fence_runtime_from(&self, expected: u64) -> Result<u64> {
        let _publication = self
            .inner
            .runtime_state
            .lock()
            .map_err(|_| Error::Storage)?;
        if self.inner.generation.load(Ordering::Acquire) != expected {
            return Err(Error::ManagerStopping);
        }
        Ok(self.fence_runtime_locked())
    }
    // Both callers hold runtime_state; the returned generation belongs to this
    // precise fence, rather than a later concurrently observed shutdown.
    fn fence_runtime_locked(&self) -> u64 {
        self.inner.startup_complete.store(false, Ordering::Release);
        let exempt = self
            .inner
            .stop_callers
            .lock()
            .map(|ids| ids.clone())
            .unwrap_or_default();
        self.inner.dispatch.fence_except(&exempt);
        self.inner.stopping.store(true, Ordering::Release);
        let generation = self
            .inner
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        if let Ok(token) = self.inner.creation_cancel.lock() {
            token.cancel();
        }
        if let Ok(tokens) = self.inner.cancellations.lock() {
            for token in tokens.values() {
                token.cancel();
            }
        }
        generation
    }
    fn cancellation(&self, id: i32) -> Result<Arc<Cancellation>> {
        if !self.protocol(|p| Ok(p.snapshot(id).is_ok()))? {
            return Ok(Arc::default());
        }
        Ok(Arc::clone(
            self.inner
                .cancellations
                .lock()
                .map_err(|_| Error::Storage)?
                .entry(id)
                .or_default(),
        ))
    }
    pub(crate) fn reserve_payload(
        &self,
        id: Option<i32>,
        amount: usize,
    ) -> Result<PayloadReservation> {
        if amount == 0 || self.inner.work.policy.is_none() {
            return Ok(PayloadReservation::empty());
        }
        let token = self.inner.reservation.fetch_add(1, Ordering::AcqRel);
        if token == 0 {
            return Err(Error::Capacity);
        }
        if let Err(error) = self.protocol(|p| p.reserve_external(id, token, amount)) {
            self.runtime_refuse(id, RuntimeRefusal::Capacity, false);
            return Err(error);
        }
        self.inner.expiry_changed.notify_one();
        let weak = Arc::downgrade(&self.inner);
        Ok(PayloadReservation::new(move || {
            if let Some(inner) = weak.upgrade()
                && let Ok(mut protocol) = inner.protocol.lock()
            {
                protocol.release_external(token);
                inner.expiry_changed.notify_one();
            }
        }))
    }
    pub(crate) fn with_operation<'a, R: Send + 'a, F: Future<Output = Result<R>> + Send + 'a>(
        &'a self,
        id: Option<i32>,
        kind: WorkKind,
        future: F,
    ) -> transport::BoxFuture<'a, Result<R>> {
        self.with_payload_operation(id, kind, 0, future)
    }
    pub(crate) fn with_payload_operation<
        'a,
        R: Send + 'a,
        F: Future<Output = Result<R>> + Send + 'a,
    >(
        &'a self,
        id: Option<i32>,
        kind: WorkKind,
        amount: usize,
        future: F,
    ) -> transport::BoxFuture<'a, Result<R>> {
        Box::pin(async move {
            if matches!(kind, WorkKind::Create | WorkKind::Accept)
                && !self.inner.accepting_requests.load(Ordering::Acquire)
            {
                return Err(Error::Paused);
            }
            let Some(policy) = self.inner.work.policy else {
                return future.await;
            };
            let mode = self.protocol(|p| Ok(p.mode()))?;
            if mode == StoreMode::Protected
                && (kind == WorkKind::Persistence || !self.inner.running.load(Ordering::Acquire))
            {
                return Err(Error::Paused);
            }
            let generation = self.generation()?;
            let operation = match kind {
                WorkKind::Receive => crate::storage::Operation::Receive,
                WorkKind::Create => crate::storage::Operation::Create,
                WorkKind::Accept => crate::storage::Operation::Accept,
                WorkKind::Retry => crate::storage::Operation::Retry,
                WorkKind::Rekey => crate::storage::Operation::Rekey,
                WorkKind::File | WorkKind::Persistence => crate::storage::Operation::File,
                WorkKind::Resume => crate::storage::Operation::Resume,
                WorkKind::Ordinary => crate::storage::Operation::Send,
            };
            if self
                .inner
                .options
                .admission
                .as_ref()
                .is_some_and(|admit| !admit(id, operation))
            {
                self.runtime_refuse(id, RuntimeRefusal::Capacity, kind == WorkKind::Receive);
                return Err(Error::Paused);
            }
            let cancellation = if kind == WorkKind::Resume {
                None
            } else if matches!(kind, WorkKind::Create | WorkKind::Accept) && id.is_none() {
                Some(Arc::clone(
                    &*self
                        .inner
                        .creation_cancel
                        .lock()
                        .map_err(|_| Error::Storage)?,
                ))
            } else {
                id.map(|id| self.cancellation(id)).transpose()?
            };
            self.check_generation(generation)?;
            if cancellation
                .as_ref()
                .is_some_and(|c| c.cancelled.load(Ordering::Acquire))
            {
                return Err(Error::Paused);
            }
            let identity = Arc::as_ptr(&self.inner) as usize;
            let already_owned = nested(identity);
            let pending: PendingIdentity = Arc::default();
            let mut deadline_hit = false;
            let permit = if already_owned {
                None
            } else {
                match self.inner.work.acquire(1) {
                    Ok(permit) => Some(Arc::new(permit)),
                    Err(error) => {
                        self.runtime_refuse(
                            id,
                            RuntimeRefusal::Capacity,
                            kind == WorkKind::Receive,
                        );
                        return Err(error);
                    }
                }
            };
            let _payload = self.reserve_payload(id, amount)?;
            let mut guard = OperationGuard {
                manager: self.clone(),
                id,
                kind,
                generation,
                mode,
                armed: true,
                creation_generation: self.inner.creation_generation.load(Ordering::Acquire),
            };
            let interrupt = async {
                if let Some(cancellation) = cancellation {
                    cancellation.wait().await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            let mut held: Vec<Arc<dyn Send + Sync>> = vec![_payload.0.clone()];
            if let Some(permit) = &permit {
                held.push(permit.clone());
            }
            let cancellable = crate::media::blocking::resources(held, async {
                tokio::select! {result=future=>Some(result),_=interrupt=>None}
            });
            let result = if already_owned {
                match cancellable.await {
                    Some(result) => result,
                    None => {
                        guard.armed = false;
                        Err(Error::Paused)
                    }
                }
            } else {
                match scope_pending(
                    identity,
                    Arc::clone(&pending),
                    tokio::time::timeout(
                        Duration::from_secs_f64(policy.operation_seconds),
                        cancellable,
                    ),
                )
                .await
                {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        guard.armed = false;
                        Err(Error::Paused)
                    }
                    Err(_) => {
                        deadline_hit = true;
                        self.runtime_refuse(
                            id,
                            RuntimeRefusal::Deadline,
                            kind == WorkKind::Receive,
                        );
                        guard.armed = mode == StoreMode::Protected
                            && id.is_none()
                            && kind == WorkKind::Create;
                        Err(Error::Paused)
                    }
                }
            };
            if result.is_ok() || mode != StoreMode::Protected {
                guard.armed = false;
            }
            drop(guard);
            drop(permit);
            self.inner.expiry_changed.notify_one();
            if deadline_hit
                && matches!(mode, StoreMode::Bounded | StoreMode::Ingress)
                && kind != WorkKind::Receive
                && let Some((chat_id, random_id)) = *pending.lock().map_err(|_| Error::Storage)?
            {
                return Err(Error::SendPending {
                    chat_id,
                    random_id,
                    cause: crate::error::PendingCause::Deadline,
                });
            }
            if mode == StoreMode::Protected && result.is_err() {
                Err(Error::Paused)
            } else {
                result
            }
        })
    }
}
struct OperationGuard<S: Storage + 'static, T: Transport> {
    manager: SecretChatManager<S, T>,
    id: Option<i32>,
    kind: WorkKind,
    generation: u64,
    creation_generation: u64,
    mode: StoreMode,
    armed: bool,
}
impl<S: Storage + 'static, T: Transport> Drop for OperationGuard<S, T> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Fences and publication use the same lock. No fence path drops an
        // operation guard: dispatch.fence_except only closes admissions.
        let _publication = self
            .manager
            .inner
            .runtime_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.manager.inner.generation.load(Ordering::Acquire) != self.generation {
            return;
        }
        if self.mode == StoreMode::Protected {
            self.manager
                .runtime_refuse(self.id, RuntimeRefusal::Cancelled, false);
            if self.id.is_none()
                && self.kind == WorkKind::Create
                && self
                    .manager
                    .inner
                    .creation_generation
                    .compare_exchange(
                        self.creation_generation,
                        self.creation_generation.wrapping_add(1),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                let result = self.manager.protocol(|p| {
                    p.transaction(|state| {
                        crate::custody::transient::suspend_requests(state);
                        Ok(())
                    })
                });
                if result.is_err() {
                    tracing::warn!("An interrupted initial request could not be suspended");
                }
            }
        } else if self.kind == WorkKind::Receive {
            self.manager
                .runtime_refuse(self.id, RuntimeRefusal::Cancelled, true);
        }
    }
}
#[cfg(test)]
mod tests;
