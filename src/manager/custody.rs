use super::*;
use crate::custody::{CustodyStorage, PauseReason, PauseStatus, TransientStorage};
pub type BoundedSecretChatManager<S, T> = SecretChatManager<CustodyStorage<S>, T>;
pub type TransientSecretChatManager<S, T> = SecretChatManager<TransientStorage<S>, T>;
impl<S: Storage + 'static, T: Transport> SecretChatManager<TransientStorage<S>, T> {
    /// Stop new create, accept and requested intake while existing chats continue.
    pub fn set_accepting_requests(&self, enabled: bool) -> Result<()> {
        self.inner
            .accepting_requests
            .store(enabled, Ordering::Release);
        Ok(())
    }
    /// Inspect local refusal without exposing key material.
    pub fn is_suspended(&self, id: i32) -> Result<bool> {
        let snapshot = self.status(id)?;
        Ok(snapshot.state == crate::chat::ChatState::Suspended
            || self
                .inner
                .cancellations
                .lock()
                .map_err(|_| Error::Storage)?
                .get(&id)
                .is_some_and(|token| token.is_cancelled()))
    }
}
impl<S: Storage + 'static, T: Transport> SecretChatManager<CustodyStorage<S>, T> {
    /// Pause one conversation without discarding its durable obligations.
    pub fn pause_chat(&self, id: i32) -> Result<()> {
        self.generation()?;
        self.protocol(|p| p.pause_custody(id, PauseReason::Operator, false))?;
        self.cancel_runtime(id);
        Ok(())
    }
    /// Inspect the storage-owned pause reason and whether continuity can resume.
    pub fn pause_status(&self, id: i32) -> Result<PauseStatus> {
        self.protocol(|p| p.custody_status(id))
    }
    /// Check age and capacity now, retaining every required record.
    pub fn check_custody(&self) -> Result<Vec<i32>> {
        let (result, paused) = self.protocol(|p| {
            let result = p.check_custody();
            // Earlier pauses may have committed before a later persistence failure.
            // The protocol adopts that committed view on both success and error.
            Ok((result, p.paused_custody_ids()))
        })?;
        for id in paused {
            self.cancel_runtime(id);
        }
        result
    }
    /// Resume intact custody and retry its original delivery/send identities.
    pub async fn resume_chat(&self, id: i32) -> Result<()> {
        self.with_operation(Some(id), runtime::WorkKind::Resume, async {
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            self.protocol(|p| p.resume_custody(id))?;
            self.inner
                .cancellations
                .lock()
                .map_err(|_| Error::Storage)?
                .remove(&id);
            self.bump_epoch(id)?;
            self.drain_deliveries(id).await?;
            self.retry_pending_locked(id).await
        })
        .await
    }
}
