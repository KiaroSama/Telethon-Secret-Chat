//! Operator controls retain storage-owned continuity metadata and cached state.
use super::*;
use crate::custody::{CustodyStorage, PauseReason, PauseStatus};
impl<S: Storage> Protocol<CustodyStorage<S>> {
    pub fn custody_status(&self, id: i32) -> Result<PauseStatus> {
        self.storage.status(id)
    }
    pub fn pause_custody(&mut self, id: i32, reason: PauseReason, lost: bool) -> Result<()> {
        let result = self.storage.pause(id, reason, lost);
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    pub fn resume_custody(&mut self, id: i32) -> Result<()> {
        let result = self.storage.resume(id);
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
    pub fn check_custody(&mut self) -> Result<Vec<i32>> {
        let result = self.storage.check_custody();
        if let Some(view) = self.storage.committed_view() {
            self.state = view;
        }
        result
    }
}
