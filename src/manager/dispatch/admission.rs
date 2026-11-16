//! A shutdown barrier includes callbacks whose factory has not returned yet.
use super::*;
use std::collections::BTreeSet;
#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    pending: BTreeSet<u64>,
}
#[derive(Default)]
pub(super) struct Admissions {
    state: Mutex<State>,
    changed: Notify,
}
pub(super) struct Ticket {
    owner: Arc<Admissions>,
    id: u64,
}
impl Admissions {
    pub fn enter(self: &Arc<Self>, stopping: bool) -> Result<Ticket> {
        let mut state = self.state.lock().map_err(|_| Error::Storage)?;
        if state.closed && !stopping {
            return Err(Error::ManagerStopping);
        }
        state.next = state.next.checked_add(1).ok_or(Error::Capacity)?;
        let id = state.next;
        state.pending.insert(id);
        Ok(Ticket {
            owner: self.clone(),
            id,
        })
    }
    pub fn closed(&self) -> bool {
        self.state.lock().map_or(true, |state| state.closed)
    }
    pub fn fence(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .closed = true;
    }
    pub fn reopen(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| Error::Storage)?;
        if !state.pending.is_empty() {
            return Err(Error::ManagerStopping);
        }
        state.closed = false;
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<u64> {
        Ok(self.state.lock().map_err(|_| Error::Storage)?.next)
    }
    pub async fn wait(&self, through: u64) -> Result<()> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self
                .state
                .lock()
                .map_err(|_| Error::Storage)?
                .pending
                .iter()
                .any(|id| *id <= through)
            {
                return Ok(());
            }
            changed.await;
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.owner
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pending
            .remove(&self.id);
        self.owner.changed.notify_waiters();
    }
}
