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
    pub(crate) hook: Arc<dyn DurableIngressHook>,
    pub(crate) permit: u64,
}
impl Drop for InvocationPermit {
    fn drop(&mut self) {
        self.hook.cancel_request(self.permit);
    }
}
#[derive(Default)]
pub(crate) struct Boundary {
    pub(crate) started: bool,
    pub(crate) receiver: bool,
    pub(crate) hook: Option<Arc<dyn DurableIngressHook>>,
}
pub(crate) type SharedBoundary = Arc<Mutex<Boundary>>;
