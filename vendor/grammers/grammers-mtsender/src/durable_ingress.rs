// Kiaro supplement to grammers 02367f97. MIT OR Apache-2.0.
//! A pre-ack boundary shared by every clone of the original sender-pool handle.
use grammers_session::updates::UpdateAndPeers;
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngressReceiptKind {
    Live,
    Difference,
    ChannelDifference,
    Machine,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IngressRefused;
impl std::fmt::Display for IngressRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("durable ingress refused the operation")
    }
}
impl std::error::Error for IngressRefused {}
impl From<IngressRefused> for crate::InvocationError {
    fn from(_: IngressRefused) -> Self {
        Self::Dropped("durable ingress refused the operation")
    }
}
/// Calls are synchronous and must complete the durable transaction before
/// returning. Implementations must fence themselves after uncertain storage.
/// They must not invoke Telegram or re-enter the sender pool from a callback.
/// Operational SDK paths await one ordered blocking worker per original pool;
/// the pre-start attachment callback still runs under its exclusive setup lock.
pub trait DurableIngressHook: Send + Sync {
    /// Called under the original pool's pre-start ownership lock. No runner or
    /// update receiver can acquire that boundary until this commit completes.
    fn prepare_attachment(&self) -> Result<(), IngressRefused> {
        self.restore().map(|_| ())
    }
    fn stage_live(
        &self,
        dc_id: i32,
        input: &grammers_session::updates::UpdatesLike,
    ) -> Result<(), IngressRefused>;
    fn pending_live(&self) -> Result<Vec<grammers_session::updates::UpdatesLike>, IngressRefused>;
    fn receipt_notification(&self) -> Arc<tokio::sync::Notify>;
    fn restore(&self) -> Result<Vec<u8>, IngressRefused>;
    fn replay(&self) -> Result<Vec<(u64, UpdateAndPeers)>, IngressRefused>;
    fn commit(
        &self,
        before: &[u8],
        after: &[u8],
        kind: IngressReceiptKind,
        envelope: &[u8],
        output: &UpdateAndPeers,
    ) -> Result<Option<u64>, IngressRefused>;
    fn guard_request(&self, dc_id: i32, body: &[u8]) -> Result<u64, IngressRefused>;
    /// Capture caller-issued difference responses too, before any caller sees
    /// them. Automatic receiver calls use this same boundary.
    fn guard_result(
        &self,
        permit: u64,
        dc_id: i32,
        request: &[u8],
        response: &[u8],
    ) -> Result<(), IngressRefused>;
    fn cancel_request(&self, permit: u64);
}
pub(crate) struct InvocationPermit {
    pub(crate) worker: DurableWorker,
    pub(crate) permit: u64,
    pub(crate) armed: bool,
}
impl InvocationPermit {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for InvocationPermit {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let permit = self.permit;
        // Queue cleanup synchronously on the same executor as every callback.
        // A caller may destroy its Tokio runtime immediately after this drop.
        let _ = self.worker.executor.send(Box::new(move |hook| {
            hook.cancel_request(permit);
        }));
    }
}
/// One ordered blocking operation per original pool. Cancellation cannot release
/// the permit while a filesystem callback is still executing on its worker.
#[derive(Clone)]
pub(crate) struct DurableWorker {
    pub(crate) hook: Arc<dyn DurableIngressHook>,
    executor: Arc<Executor>,
}
struct Executor {
    queue: Option<std::sync::mpsc::Sender<Work>>,
    serial: Arc<tokio::sync::Semaphore>,
    thread: Option<std::thread::JoinHandle<()>>,
}
type Work = Box<dyn FnOnce(&dyn DurableIngressHook) + Send + 'static>;
impl Executor {
    fn send(self: &Arc<Self>, operation: Work) -> Result<(), IngressRefused> {
        let owner = Arc::clone(self);
        self.queue
            .as_ref()
            .ok_or(IngressRefused)?
            .send(Box::new(move |hook| {
                // In-flight work owns the executor, so dropping a cancelled
                // caller never joins a thread still performing filesystem I/O.
                let _owner = owner;
                operation(hook);
            }))
            .map_err(|_| IngressRefused)
    }
}
impl Drop for Executor {
    fn drop(&mut self) {
        self.queue.take();
        if let Some(thread) = self.thread.take()
            && thread.thread().id() != std::thread::current().id()
        {
            // With no queued work this releases the idle hook and its storage
            // handles before the pool's last owner finishes dropping.
            let _ = thread.join();
        }
    }
}
impl DurableWorker {
    pub(crate) fn new(hook: Arc<dyn DurableIngressHook>) -> Result<Self, IngressRefused> {
        let (queue, receive) = std::sync::mpsc::channel::<Work>();
        let working_hook = Arc::clone(&hook);
        let thread = std::thread::Builder::new()
            .name("grammers-durable-ingress".into())
            .spawn(move || {
                for operation in receive {
                    operation(working_hook.as_ref());
                }
            })
            .map_err(|_| IngressRefused)?;
        Ok(Self {
            hook,
            executor: Arc::new(Executor {
                queue: Some(queue),
                serial: Arc::new(tokio::sync::Semaphore::new(1)),
                thread: Some(thread),
            }),
        })
    }
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&dyn DurableIngressHook) -> Result<T, IngressRefused> + Send + 'static,
    ) -> Result<T, IngressRefused> {
        let permit = Arc::clone(&self.executor.serial)
            .acquire_owned()
            .await
            .map_err(|_| IngressRefused)?;
        let (send, response) = tokio::sync::oneshot::channel();
        let executor = Arc::clone(&self.executor);
        self.executor
            .queue
            .as_ref()
            .ok_or(IngressRefused)?
            .send(Box::new(move |hook| {
                let result = operation(hook);
                drop(permit);
                // Successful completion transfers the work's executor owner
                // to the caller. It must not remain on this thread after the
                // caller resumes and drops its last public pool handle.
                let _ = send.send((result, executor));
            }))
            .map_err(|_| IngressRefused)?;
        let (result, _executor) = response.await.map_err(|_| IngressRefused)?;
        result
    }
}
pub(crate) struct Boundary {
    pub(crate) started: bool,
    pub(crate) dispatched: bool,
    pub(crate) receiver: bool,
    pub(crate) hook: Option<Arc<dyn DurableIngressHook>>,
    pub(crate) executor: Option<DurableWorker>,
}
impl Default for Boundary {
    fn default() -> Self {
        Self {
            started: false,
            dispatched: false,
            receiver: false,
            hook: None,
            executor: None,
        }
    }
}
impl Boundary {
    pub(crate) fn worker(&self) -> Option<DurableWorker> {
        self.executor.clone()
    }
}
pub(crate) type SharedBoundary = Arc<Mutex<Boundary>>;
