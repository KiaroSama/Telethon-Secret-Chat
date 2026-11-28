//! Whole-state commits keep counters, keys and retained bytes in one transaction.
#[cfg(test)]
mod audit_tests;
#[cfg(test)]
mod contract_tests;
mod directory;
#[cfg(test)]
mod foundation;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod lifecycle_tests;
pub use crate::secret_record::SecretRecord;
use crate::{
    chat::Chat,
    error::{Error, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;
pub const STORE_VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreMode {
    #[default]
    Durable,
    Bounded,
    Protected,
    Ingress,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub mode: StoreMode,
    pub authorization: String,
    pub chats: BTreeMap<i32, Chat>,
    pub settings: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub private_records: BTreeMap<String, SecretRecord>,
}
impl State {
    pub fn empty(authorization: String) -> Result<Self> {
        if authorization.len() != 64 || !authorization.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Authorization);
        }
        Ok(Self {
            version: STORE_VERSION,
            mode: StoreMode::Durable,
            authorization,
            chats: BTreeMap::new(),
            settings: BTreeMap::new(),
            private_records: BTreeMap::new(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_with_context(false)
    }
    pub(crate) fn validate_restored(&self) -> Result<()> {
        self.validate_with_context(true)
    }
    fn validate_with_context(&self, context: bool) -> Result<()> {
        if self.version != STORE_VERSION
            || self.authorization.len() != 64
            || !self.authorization.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::InvalidState);
        }
        for (id, chat) in &self.chats {
            if *id != chat.id {
                return Err(if context {
                    Error::InvalidState.with_chat(*id)
                } else {
                    Error::InvalidState
                });
            }
            chat.validate()
                .map_err(|error| if context { error.with_chat(*id) } else { error })?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    Accept,
    Send,
    Receive,
    Retry,
    Rekey,
    File,
    Resume,
    Recovery,
    Bookkeeping,
    Cleanup,
}
#[derive(Debug, Clone, Copy)]
pub struct RuntimePolicy {
    pub operation_seconds: f64,
    pub cleanup_seconds: f64,
    pub tasks: usize,
    pub bytes: usize,
    pub chats: usize,
    pub records: usize,
    pub frame_bytes: usize,
}
#[derive(Debug, Clone, Copy)]
pub enum RuntimeRefusal {
    Capacity,
    Deadline,
    Cancelled,
    Callback,
}
/// Expired owners must be cancelled even if persisting their safe state failed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuntimeExpiry {
    pub chats: Vec<i32>,
    pub unbound: bool,
    pub persistence_failed: bool,
}
pub trait Storage: Send {
    /// A backend can request one validated recovery commit before participation.
    fn repair_pending(&self) -> bool {
        false
    }
    fn admit_budget(&mut self, _chat: i32, _records: usize, _bytes: usize) -> Result<()> {
        Ok(())
    }
    /// Earliest absolute Unix millisecond deadline, excluding expired owners.
    fn next_runtime_expiry_ms(&self) -> Result<Option<u64>> {
        Ok(None)
    }
    fn expire_runtime(&mut self) -> Result<RuntimeExpiry> {
        Ok(RuntimeExpiry::default())
    }
    fn reserve_external(&mut self, _chat: Option<i32>, _token: u64, _bytes: usize) -> Result<()> {
        Ok(())
    }
    fn release_external(&mut self, _token: u64) {}
    fn runtime_policy(&self) -> Option<RuntimePolicy> {
        None
    }
    fn runtime_refused(&mut self, _id: i32, _reason: RuntimeRefusal, _lost: bool) -> Result<()> {
        Ok(())
    }
    fn suspend_storage(&mut self) -> Result<()> {
        Ok(())
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        self.load()
    }
    fn admit(&mut self, _chat: Option<i32>, _operation: Operation, _bytes: usize) -> Result<()> {
        Ok(())
    }
    fn default_auto_save_folder(&self) -> Option<PathBuf> {
        None
    }
    fn mark_nonresumable(&mut self, _chat: i32) -> Result<()> {
        Ok(())
    }
    fn note_attempt(&mut self, _chat: i32, _wire_sequence: i32) -> Result<()> {
        Ok(())
    }
    fn load(&self) -> Result<Option<State>>;
    /// On any error the backend must either retain its previous snapshot or
    /// refuse subsequent operations if durable visibility is uncertain.
    fn commit(&mut self, state: &State) -> Result<()>;
    /// Wrappers that attach transactional control metadata can publish the exact
    /// committed view without forcing ordinary backends to re-read the file.
    fn committed_view(&self) -> Option<State> {
        None
    }
}
#[derive(Default)]
pub struct MemoryStorage {
    state: Option<State>,
}
impl Storage for MemoryStorage {
    fn load(&self) -> Result<Option<State>> {
        Ok(self.state.clone())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        state.validate()?;
        self.state = Some(state.clone());
        Ok(())
    }
}
pub struct FileStorage {
    path: PathBuf,
    _lock: Option<File>,
    owner_pid: u32,
    poisoned: bool,
    repair_pending: AtomicBool,
    #[cfg(test)]
    commit_fault: Option<CommitFault>,
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum CommitFault {
    BeforeReplace,
    AfterReplace,
    #[cfg(unix)]
    DirectorySync,
}
impl FileStorage {
    #[cfg(unix)]
    fn directory_sync_failed(&mut self, mode: StoreMode) -> Result<()> {
        if mode == StoreMode::Durable {
            tracing::warn!("Directory durability could not be confirmed");
            self.repair_pending.store(false, Ordering::Release);
            Ok(())
        } else {
            self.poisoned = true;
            Err(Error::Storage)
        }
    }
    #[cfg(test)]
    pub(crate) fn fail_next_commit(&mut self, fault: CommitFault) {
        self.commit_fault = Some(fault);
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = std::path::absolute(path).map_err(|_| Error::Storage)?;
        let parent = path.parent().ok_or(Error::Storage)?;
        directory::create_parent(parent)?;
        let parent = parent.canonicalize().map_err(|_| Error::Storage)?;
        let path = parent.join(path.file_name().ok_or(Error::Storage)?);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
            return Err(Error::Storage);
        }
        let mut lockname = path.file_name().ok_or(Error::Storage)?.to_os_string();
        lockname.push(".lock");
        let lockpath = path.with_file_name(lockname);
        if fs::symlink_metadata(&lockpath).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
        {
            return Err(Error::Storage);
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let lock = options.open(lockpath).map_err(|_| Error::Storage)?;
        lock.try_lock().map_err(|_| Error::Storage)?;
        crate::temporary::cleanup(
            path.parent().ok_or(Error::Storage)?,
            &temporary_prefix(&path)?,
        );
        Ok(Self {
            path,
            _lock: Some(lock),
            owner_pid: std::process::id(),
            poisoned: false,
            repair_pending: AtomicBool::new(false),
            #[cfg(test)]
            commit_fault: None,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn release_lock(&mut self) -> Result<()> {
        if self.owner_pid != std::process::id() {
            return Err(Error::Storage);
        }
        if let Some(lock) = self._lock.as_ref() {
            // A concurrent child may retain the same open-file description
            // until exec closes CLOEXEC descriptors. Release the owner's lock
            // explicitly rather than waiting for the final inherited close.
            lock.unlock().map_err(|_| Error::Storage)?;
        }
        self._lock.take();
        Ok(())
    }
}
impl Drop for FileStorage {
    fn drop(&mut self) {
        // A forked child may close its handle but must never unlock its parent.
        let _ = self.release_lock();
    }
}
impl Storage for FileStorage {
    fn repair_pending(&self) -> bool {
        self.repair_pending.load(Ordering::Acquire)
    }
    fn suspend_storage(&mut self) -> Result<()> {
        self.release_lock()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        if self.poisoned || self.owner_pid != std::process::id() {
            return Err(Error::Storage);
        }
        if self._lock.is_none() {
            let mut reopened = FileStorage::open(&self.path)?;
            self._lock = reopened._lock.take();
        }
        self.load()
    }
    fn default_auto_save_folder(&self) -> Option<PathBuf> {
        self.path
            .parent()
            .map(|parent| parent.join("saved-secret-chats"))
    }
    fn load(&self) -> Result<Option<State>> {
        let Some(mut state) = self.read_snapshot()? else {
            return Ok(None);
        };
        let mut repaired = false;
        // Old ordinary snapshots could retain material after becoming terminal.
        // Protected custody is never eligible for this destructive recovery.
        if state.mode == StoreMode::Durable {
            for chat in state
                .chats
                .values_mut()
                .filter(|chat| chat.state == crate::chat::ChatState::Closed)
            {
                if chat.key.is_some()
                    || chat.previous_key.is_some()
                    || chat.exchange.is_some()
                    || chat.handshake_secret.is_some()
                    || chat.peer_public.is_some()
                    || chat.dh.is_some()
                    || !chat.outbox.is_empty()
                    || !chat.gaps.is_empty()
                    || !chat.deliveries.is_empty()
                    || !chat.service_deliveries.is_empty()
                    || chat.gap_end.is_some()
                    || chat.resend_due.is_some()
                {
                    chat.close();
                    repaired = true;
                }
            }
        }
        state.validate_restored()?;
        self.repair_pending.store(repaired, Ordering::Release);
        Ok(Some(state))
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        self.commit_with_sync(state, File::sync_all)
    }
}
impl FileStorage {
    pub(crate) fn read_snapshot(&self) -> Result<Option<State>> {
        if self.poisoned || self._lock.is_none() || self.owner_pid != std::process::id() {
            return Err(Error::Storage);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = match options.open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Storage),
        };
        let metadata = file.metadata().map_err(|_| Error::Storage)?;
        if metadata.len() > MAX_STORE_BYTES {
            return Err(Error::InvalidState);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::Storage);
            }
        }
        let mut bytes = Zeroizing::new(Vec::new());
        Read::by_ref(&mut file)
            .take(MAX_STORE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Storage)?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::InvalidState);
        }
        decode_state(&bytes).map(Some)
    }

    fn commit_with_sync(
        &mut self,
        state: &State,
        sync: impl FnOnce(&File) -> std::io::Result<()>,
    ) -> Result<()> {
        if self.poisoned || self._lock.is_none() || self.owner_pid != std::process::id() {
            return Err(Error::Storage);
        }
        state.validate()?;
        let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::Storage)?);
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::Capacity);
        }
        let parent = self.path.parent().ok_or(Error::Storage)?;
        let mut temporary = crate::temporary::create(parent, &temporary_prefix(&self.path)?)?;
        temporary.write_all(&bytes).map_err(|_| Error::Storage)?;
        sync(temporary.as_file()).map_err(|_| Error::Storage)?;
        #[cfg(test)]
        let commit_fault = self.commit_fault.take();
        #[cfg(test)]
        if matches!(commit_fault, Some(CommitFault::BeforeReplace)) {
            return Err(Error::Storage);
        }
        // After replacement failure/uncertain directory sync, do not keep using
        // a RAM snapshot that might disagree with the filesystem.
        if temporary.persist(&self.path).is_err() {
            self.poisoned = true;
            return Err(Error::Storage);
        }
        #[cfg(test)]
        if matches!(commit_fault, Some(CommitFault::AfterReplace)) {
            self.poisoned = true;
            return Err(Error::Storage);
        }
        #[cfg(all(test, unix))]
        if matches!(commit_fault, Some(CommitFault::DirectorySync)) {
            return self.directory_sync_failed(state.mode);
        }
        #[cfg(unix)]
        if File::open(parent).and_then(|f| f.sync_all()).is_err() {
            return self.directory_sync_failed(state.mode);
        }
        self.repair_pending.store(false, Ordering::Release);
        Ok(())
    }
}

fn temporary_prefix(path: &Path) -> Result<String> {
    use sha2::Digest;
    let name = path.file_name().ok_or(Error::Storage)?;
    let digest = sha2::Sha256::digest(name.as_encoded_bytes());
    Ok(format!(
        ".secret-chat-store-{}-",
        hex::encode(&digest[..12])
    ))
}
fn decode_state(bytes: &[u8]) -> Result<State> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let state = serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
        use serde_path_to_error::Segment;
        let mut path = error.path().iter();
        match (path.next(), path.next()) {
            (Some(Segment::Map { key: group }), Some(Segment::Map { key: id }))
                if group == "chats" =>
            {
                match id.parse::<i32>().ok().filter(|id| *id != 0) {
                    Some(id) => Error::InvalidState.with_chat(id),
                    None => Error::InvalidState,
                }
            }
            _ => Error::InvalidState,
        }
    })?;
    decoder.end().map_err(|_| Error::InvalidState)?;
    Ok(state)
}
