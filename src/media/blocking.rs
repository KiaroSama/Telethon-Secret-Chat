//! Ownership for bounded blocking I/O, including cancellation and shutdown joins.
use crate::error::{Error, Result};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::Notify;
type Hold = Arc<dyn Send + Sync>;
tokio::task_local! {static TRACKER: Arc<IoTracker>; static HOLDS: RefCell<Vec<Weak<dyn Send+Sync>>>;}
#[derive(Default)]
struct Job {
    cancelled: AtomicBool,
    finished: AtomicBool,
    changed: Notify,
}
impl Job {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }
    async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}
#[derive(Default)]
struct Jobs {
    fenced: bool,
    active: Vec<Arc<Job>>,
}
#[derive(Default)]
pub(crate) struct IoTracker(Mutex<Jobs>);
impl IoTracker {
    fn register(&self, job: &Arc<Job>) -> Result<()> {
        let mut jobs = self.0.lock().map_err(|_| Error::Storage)?;
        if jobs.fenced {
            return Err(Error::ManagerStopping);
        }
        jobs.active
            .retain(|job| !job.finished.load(Ordering::Acquire));
        jobs.active.push(Arc::clone(job));
        Ok(())
    }
    pub(crate) async fn scope<T, F: Future<Output = Result<T>>>(
        self: &Arc<Self>,
        future: F,
    ) -> Result<T> {
        let job = Arc::new(Job::default());
        self.register(&job)?;
        let completion = Completion {
            job: Arc::clone(&job),
            holds: Vec::new(),
        };
        let result=TRACKER.scope(Arc::clone(self),async {
            tokio::select! {biased;_=job.cancelled()=>Err(Error::ManagerStopping),result=future=>result}
        }).await;
        drop(completion);
        result
    }
    pub(crate) fn resume(&self) -> Result<()> {
        let mut jobs = self.0.lock().map_err(|_| Error::Storage)?;
        jobs.active
            .retain(|job| !job.finished.load(Ordering::Acquire));
        if !jobs.active.is_empty() {
            return Err(Error::ManagerStopping);
        }
        jobs.fenced = false;
        Ok(())
    }
    pub(crate) async fn drain(&self) -> Result<()> {
        let jobs = {
            let mut jobs = self.0.lock().map_err(|_| Error::Storage)?;
            jobs.fenced = true;
            jobs.active.clone()
        };
        for job in &jobs {
            job.cancel();
        }
        for job in jobs {
            loop {
                let changed = job.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if job.finished.load(Ordering::Acquire) {
                    break;
                }
                changed.await;
            }
        }
        self.0
            .lock()
            .map_err(|_| Error::Storage)?
            .active
            .retain(|job| !job.finished.load(Ordering::Acquire));
        Ok(())
    }
}
pub(crate) async fn resources<F: Future>(mut initial: Vec<Hold>, future: F) -> F::Output {
    if let Ok(outer) = HOLDS.try_with(|holds| {
        holds
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    }) {
        initial.extend(outer);
    }
    let weak = initial.iter().map(Arc::downgrade).collect();
    let result = HOLDS.scope(RefCell::new(weak), future).await;
    drop(initial);
    result
}
pub(crate) fn retain<T: Send + Sync + 'static>(resource: &Arc<T>) {
    let resource: Hold = resource.clone();
    let _ = HOLDS.try_with(|holds| {
        let mut holds = holds.borrow_mut();
        holds.retain(|hold| hold.strong_count() != 0);
        holds.push(Arc::downgrade(&resource));
    });
}
struct Completion {
    job: Arc<Job>,
    holds: Vec<Hold>,
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.holds.clear();
        self.job.finished.store(true, Ordering::Release);
        self.job.changed.notify_waiters();
    }
}
pub(crate) struct Blocking<T> {
    handle: tokio::task::JoinHandle<OwnedResult<T>>,
    job: Arc<Job>,
}
struct OwnedResult<T> {
    result: T,
    _completion: Completion,
}
impl<T> Future for Blocking<T> {
    type Output = Result<T>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.handle).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(result)) => Poll::Ready(Ok(result.result)),
            Poll::Ready(Err(_)) => Poll::Ready(Err(Error::Storage)),
        }
    }
}
impl<T> Drop for Blocking<T> {
    fn drop(&mut self) {
        self.job.cancel();
    }
}
pub(crate) fn spawn<T: Send + 'static>(
    work: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<Blocking<T>> {
    let job = Arc::new(Job::default());
    if let Ok(tracker) = TRACKER.try_with(Arc::clone) {
        tracker.register(&job)?;
    }
    let holds = HOLDS
        .try_with(|holds| holds.borrow().iter().filter_map(Weak::upgrade).collect())
        .unwrap_or_default();
    let completion = Completion {
        job: Arc::clone(&job),
        holds,
    };
    let worker_job = Arc::clone(&job);
    // The actual worker and its result own completion, including when its
    // awaiter or runtime disappears. An async monitor is not a resource owner.
    let handle = tokio::task::spawn_blocking(move || {
        let result = work(&worker_job.cancelled);
        OwnedResult {
            result,
            _completion: completion,
        }
    });
    Ok(Blocking { handle, job })
}
