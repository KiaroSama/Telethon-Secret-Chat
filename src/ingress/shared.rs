use super::{application, coordinator::DurableIngress};
use crate::{
    error::{Error, Result},
    storage::{State, Storage},
};
use std::sync::Arc;
/// Single secret-protocol writer sharing the journal's actual transaction.
pub struct IngressProtocolStorage {
    pub(super) coordinator: Arc<DurableIngress>,
}
impl Storage for IngressProtocolStorage {
    fn load(&self) -> Result<Option<State>> {
        let core = self.coordinator.core.lock().map_err(|_| Error::Storage)?;
        if core.released || core.failed {
            return Err(Error::Storage);
        }
        Ok(Some(core.state.clone()))
    }
    fn committed_view(&self) -> Option<State> {
        self.coordinator
            .core
            .lock()
            .ok()
            .filter(|core| !core.failed && !core.released)
            .map(|core| core.state.clone())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        let mut core = self.coordinator.core.lock().map_err(|_| Error::Storage)?;
        if core.released
            || state.authorization != core.state.authorization
            || state.mode != core.state.mode
        {
            return Err(Error::Storage);
        }
        let mut ledger = core.ledger.clone();
        if let Some(application) = application::current(self.coordinator.owner) {
            ledger.mark(application.batch, application.index, &application.hash)?;
        }
        core.persist(state.clone(), ledger)
    }
    fn suspend_storage(&mut self) -> Result<()> {
        let mut core = self.coordinator.core.lock().map_err(|_| Error::Storage)?;
        core.stopped = true;
        core.proof = None;
        core.backend.suspend_storage()?;
        core.released = true;
        Ok(())
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        let core = self.coordinator.core.lock().map_err(|_| Error::Storage)?;
        if core.stopped || core.released || core.failed {
            return Err(Error::IngressUnavailable);
        }
        Ok(Some(core.state.clone()))
    }
}
