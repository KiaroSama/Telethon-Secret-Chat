//! Whole-state commits keep counters, keys and retained bytes in one transaction.
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
};
use zeroize::Zeroizing;
pub const STORE_VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub authorization: String,
    pub chats: BTreeMap<i32, Chat>,
    pub settings: BTreeMap<String, serde_json::Value>,
}
impl State {
    pub fn empty(authorization: String) -> Result<Self> {
        if authorization.len() != 64 || !authorization.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Authorization);
        }
        Ok(Self {
            version: STORE_VERSION,
            authorization,
            chats: BTreeMap::new(),
            settings: BTreeMap::new(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != STORE_VERSION
            || self.authorization.len() != 64
            || !self.authorization.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::InvalidState);
        }
        for (id, chat) in &self.chats {
            if *id != chat.id {
                return Err(Error::InvalidState);
            }
            chat.validate()?;
        }
        Ok(())
    }
}
pub trait Storage: Send {
    fn load(&self) -> Result<Option<State>>;
    /// On any error the backend must either retain its previous snapshot or
    /// refuse subsequent operations if durable visibility is uncertain.
    fn commit(&mut self, state: &State) -> Result<()>;
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
    _lock: File,
    poisoned: bool,
}
impl FileStorage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = std::path::absolute(path).map_err(|_| Error::Storage)?;
        let parent = path.parent().ok_or(Error::Storage)?;
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
        Ok(Self {
            path,
            _lock: lock,
            poisoned: false,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Storage for FileStorage {
    fn load(&self) -> Result<Option<State>> {
        if self.poisoned {
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
        let state: State = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidState)?;
        state.validate()?;
        Ok(Some(state))
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        if self.poisoned {
            return Err(Error::Storage);
        }
        state.validate()?;
        let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::Storage)?);
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::Capacity);
        }
        let parent = self.path.parent().ok_or(Error::Storage)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".secret-chat-store-")
            .tempfile_in(parent)
            .map_err(|_| Error::Storage)?;
        temporary.write_all(&bytes).map_err(|_| Error::Storage)?;
        temporary.as_file().sync_all().map_err(|_| Error::Storage)?;
        // After replacement failure/uncertain directory sync, do not keep using
        // a RAM snapshot that might disagree with the filesystem.
        if temporary.persist(&self.path).is_err() {
            self.poisoned = true;
            return Err(Error::Storage);
        }
        #[cfg(unix)]
        if File::open(parent).and_then(|f| f.sync_all()).is_err() {
            self.poisoned = true;
            return Err(Error::Storage);
        }
        Ok(())
    }
}
