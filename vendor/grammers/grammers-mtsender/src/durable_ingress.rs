// Kiaro supplement to grammers e61cf858. MIT OR Apache-2.0.
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
        let worker = self.worker.clone();
        let permit = self.permit;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = worker
                    .run(move |hook| {
                        hook.cancel_request(permit);
                        Ok(())
                    })
                    .await;
            });
        } else {
            worker.hook.cancel_request(permit);
        }
    }
}
/// One ordered blocking operation per original pool. Cancellation cannot release
/// the permit while a filesystem callback is still executing on its worker.
#[derive(Clone)]
pub(crate) struct DurableWorker {
    pub(crate) hook: Arc<dyn DurableIngressHook>,
    serial: Arc<tokio::sync::Semaphore>,
}
impl DurableWorker {
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&dyn DurableIngressHook) -> Result<T, IngressRefused> + Send + 'static,
    ) -> Result<T, IngressRefused> {
        let permit = Arc::clone(&self.serial)
            .acquire_owned()
            .await
            .map_err(|_| IngressRefused)?;
        let hook = Arc::clone(&self.hook);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(hook.as_ref())
        })
        .await
        .map_err(|_| IngressRefused)?
    }
}
pub(crate) struct Boundary {
    pub(crate) started: bool,
    pub(crate) dispatched: bool,
    pub(crate) receiver: bool,
    pub(crate) hook: Option<Arc<dyn DurableIngressHook>>,
    serial: Arc<tokio::sync::Semaphore>,
}
impl Default for Boundary {
    fn default() -> Self {
        Self {
            started: false,
            dispatched: false,
            receiver: false,
            hook: None,
            serial: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}
impl Boundary {
    pub(crate) fn worker(&self) -> Option<DurableWorker> {
        self.hook.as_ref().map(|hook| DurableWorker {
            hook: Arc::clone(hook),
            serial: Arc::clone(&self.serial),
        })
    }
}
pub(crate) type SharedBoundary = Arc<Mutex<Boundary>>;
