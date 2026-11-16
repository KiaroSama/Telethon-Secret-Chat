use super::*;
use crate::{
    export::{self, ExportError, ExportResult, Settings, User},
    storage::StoreMode,
};
use std::path::Path;
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    /// Export saved copies offline, including after the protocol chat was removed.
    pub fn export_saved_messages(
        &self,
        id: i32,
        destination: &Path,
        self_user: &User,
        peer_user: &User,
        options: &Settings,
    ) -> export::Result<ExportResult> {
        if self
            .protocol(|p| Ok(p.mode()))
            .map_err(|_| ExportError::Filesystem)?
            == StoreMode::Protected
        {
            return Err(ExportError::Unsupported);
        }
        let archive = self
            .inner
            .autosave
            .lock()
            .map_err(|_| ExportError::Filesystem)?
            .clone();
        let records = archive.read(id).map_err(|_| ExportError::Filesystem)?;
        export::export_saved_records(
            &records,
            destination,
            self_user,
            peer_user,
            options,
            archive.saved_in(),
        )
    }
}
