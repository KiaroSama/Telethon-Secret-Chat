use crate::{
    error::{Error, Result},
    media::{MediaReference, SourceReader},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};
type ChatLocks = BTreeMap<PathBuf, Weak<Mutex<()>>>;
static CHAT_LOCKS: LazyLock<Mutex<ChatLocks>> = LazyLock::new(Mutex::default);
// Independently loaded settings and cloned archive workers must share the same
// transaction lock for the same physical conversation directory.
fn chat_lock(root: &Path, id: i32) -> Result<Arc<Mutex<()>>> {
    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::path::absolute(root).map_err(|_| Error::Storage)?
        }
        Err(_) => return Err(Error::Storage),
    };
    let mut locks = CHAT_LOCKS.lock().map_err(|_| Error::Storage)?;
    locks.retain(|_, lock| lock.strong_count() != 0);
    let slot = locks.entry(root.join(id.to_string())).or_default();
    if let Some(lock) = slot.upgrade() {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    *slot = Arc::downgrade(&lock);
    Ok(lock)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingFile {
    pub reference: Value,
    pub name: String,
}
#[derive(Clone, Default)]
pub struct AutoSave {
    folder: Option<PathBuf>,
    saved_in: Option<PathBuf>,
}
impl AutoSave {
    /// Resolve paths when settings are installed; changing cwd cannot redirect a copy.
    pub fn load(setting: Option<Value>, default: Option<&Path>) -> Result<(Self, Option<Value>)> {
        if let Some(setting) = setting {
            let object = setting.as_object().ok_or(Error::InvalidState)?;
            let on = object
                .get("on")
                .and_then(Value::as_bool)
                .ok_or(Error::InvalidState)?;
            let saved_in = match object.get("folder") {
                Some(Value::String(path)) => {
                    Some(std::path::absolute(path).map_err(|_| Error::Storage)?)
                }
                Some(Value::Null) | None => None,
                _ => return Err(Error::InvalidState),
            };
            let folder = if on { saved_in.clone() } else { None };
            Ok((Self { folder, saved_in }, None))
        } else if let Some(path) = default {
            let path = std::path::absolute(path).map_err(|_| Error::Storage)?;
            let setting = json!({"folder":path.to_str().ok_or(Error::Storage)?,"on":true});
            Ok((
                Self {
                    folder: Some(path.clone()),
                    saved_in: Some(path),
                },
                Some(setting),
            ))
        } else {
            Ok((Self::default(), None))
        }
    }
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }
    pub fn saved_in(&self) -> Option<&Path> {
        self.saved_in.as_deref()
    }
    pub fn enabled(folder: &Path) -> Result<(Self, Value)> {
        let path = std::path::absolute(folder).map_err(|_| Error::Storage)?;
        let setting = json!({"folder":path.to_str().ok_or(Error::Storage)?,"on":true});
        Ok((
            Self {
                folder: Some(path.clone()),
                saved_in: Some(path),
            },
            setting,
        ))
    }
    pub fn disabled(&self) -> (Self, Value) {
        (
            Self {
                folder: None,
                saved_in: self.saved_in.clone(),
            },
            json!({"folder":self.saved_in.as_ref().and_then(|p|p.to_str()),"on":false}),
        )
    }
    fn chat_dir(&self, id: i32) -> Result<PathBuf> {
        let root = self.folder.as_ref().ok_or(Error::Storage)?;
        let directory = root.join(id.to_string());
        fs::create_dir_all(&directory).map_err(|_| Error::Storage)?;
        restrict_directory(&directory)?;
        Ok(directory)
    }
    pub fn files_dir(&self, id: i32) -> Result<PathBuf> {
        let directory = self.chat_dir(id)?.join("files");
        fs::create_dir_all(&directory).map_err(|_| Error::Storage)?;
        restrict_directory(&directory)?;
        Ok(directory)
    }
    pub fn append(&self, id: i32, record: &Value) -> Result<()> {
        if self.folder.is_none() {
            return Ok(());
        }
        let directory = self.chat_dir(id)?;
        let lock = chat_lock(self.folder.as_deref().ok_or(Error::Storage)?, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        self.append_unlocked(&directory, record)
    }
    fn append_unlocked(&self, directory: &Path, record: &Value) -> Result<()> {
        let path = directory.join("messages.jsonl");
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options.open(path).map_err(|_| Error::Storage)?;
        let mut encoded = serde_json::to_vec(record).map_err(|_| Error::Storage)?;
        encoded.push(b'\n');
        file.write_all(&encoded)
            .and_then(|()| file.sync_all())
            .map_err(|_| Error::Storage)
    }
    pub fn read(&self, id: i32) -> Result<Vec<Value>> {
        let Some(folder) = &self.saved_in else {
            return Ok(Vec::new());
        };
        let lock = chat_lock(folder, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        let path = folder.join(id.to_string()).join("messages.jsonl");
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(Error::Storage),
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| Error::InvalidState)?;
        let mut records: Vec<Value> = Vec::new();
        let mut indexes = BTreeMap::new();
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(id) = record.get("id") else { continue };
            let identity = serde_json::to_string(id).map_err(|_| Error::InvalidState)?;
            if record.get("type").and_then(Value::as_str) == Some("file") {
                if let Some(index) = indexes.get(&identity).copied() {
                    let original: &mut Value = &mut records[index];
                    if let Some(object) = original.as_object_mut() {
                        object.insert(
                            "file".into(),
                            record.get("path").cloned().unwrap_or(Value::Null),
                        );
                    }
                }
            } else if let Some(index) = indexes.get(&identity).copied() {
                records[index] = record;
            } else {
                indexes.insert(identity, records.len());
                records.push(record);
            }
        }
        Ok(records)
    }
    pub fn delete(&self, id: i32) -> Result<()> {
        if let Some(folder) = &self.saved_in {
            let lock = chat_lock(folder, id)?;
            let _guard = lock.lock().map_err(|_| Error::Storage)?;
            let path = folder.join(id.to_string());
            match fs::remove_dir_all(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Storage),
            }
        }
        Ok(())
    }
    pub fn owe(
        &self,
        id: i32,
        random_id: i64,
        reference: &MediaReference,
        name: &str,
    ) -> Result<()> {
        if self.folder.is_none() {
            return Ok(());
        }
        self.chat_dir(id)?;
        let lock = chat_lock(self.folder.as_deref().ok_or(Error::Storage)?, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        let mut owed = self.pending_unlocked(id)?;
        owed.insert(
            random_id.to_string(),
            PendingFile {
                reference: reference.to_json()?,
                name: safe_file_name(name),
            },
        );
        self.write_pending(id, &owed)
    }
    pub fn settle(&self, id: i32, random_id: i64) -> Result<()> {
        self.chat_dir(id)?;
        let lock = chat_lock(self.folder.as_deref().ok_or(Error::Storage)?, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        self.settle_unlocked(id, random_id)
    }
    fn settle_unlocked(&self, id: i32, random_id: i64) -> Result<()> {
        let mut owed = self.pending_unlocked(id)?;
        if owed.remove(&random_id.to_string()).is_some() {
            self.write_pending(id, &owed)?;
        }
        Ok(())
    }
    pub fn pending(&self, id: i32) -> Result<BTreeMap<String, PendingFile>> {
        self.chat_dir(id)?;
        let lock = chat_lock(self.folder.as_deref().ok_or(Error::Storage)?, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        self.pending_unlocked(id)
    }
    fn pending_unlocked(&self, id: i32) -> Result<BTreeMap<String, PendingFile>> {
        let path = self.chat_dir(id)?.join("pending.json");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| Error::InvalidState),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(_) => Err(Error::Storage),
        }
    }
    fn write_pending(&self, id: i32, owed: &BTreeMap<String, PendingFile>) -> Result<()> {
        let directory = self.chat_dir(id)?;
        let path = directory.join("pending.json");
        if owed.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Storage),
            }
            return Ok(());
        }
        crate::temporary::cleanup(&directory, ".saved-pending-");
        let mut temporary = crate::temporary::create(&directory, ".saved-pending-")?;
        let bytes = serde_json::to_vec(owed).map_err(|_| Error::Storage)?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| Error::Storage)?;
        temporary.persist(&path)?;
        Ok(())
    }
    pub fn owed_downloads(&self) -> Result<Vec<(i32, i64, MediaReference, String)>> {
        let Some(folder) = &self.folder else {
            return Ok(Vec::new());
        };
        let directories = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(Error::Storage),
        };
        let mut result = Vec::new();
        for entry in directories {
            let entry = entry.map_err(|_| Error::Storage)?;
            if !entry.file_type().map_err(|_| Error::Storage)?.is_dir() {
                continue;
            }
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            for (random, owed) in self.pending(id)? {
                let random = random.parse().map_err(|_| Error::InvalidState)?;
                result.push((
                    id,
                    random,
                    MediaReference::from_json(owed.reference)?,
                    owed.name,
                ));
            }
        }
        Ok(result)
    }
    pub fn completed_file(&self, id: i32, random_id: i64, path: &Path) -> Result<()> {
        if self.folder.is_none() {
            return Ok(());
        }
        let directory = self.chat_dir(id)?;
        let lock = chat_lock(self.folder.as_deref().ok_or(Error::Storage)?, id)?;
        let _guard = lock.lock().map_err(|_| Error::Storage)?;
        self.append_unlocked(&directory,&json!({"v":1,"type":"file","id":random_id,"path":path.to_str().ok_or(Error::Storage)?}))?;
        self.settle_unlocked(id, random_id)
    }
    pub fn copy_source(
        &self,
        id: i32,
        random_id: i64,
        source: &mut SourceReader,
    ) -> Result<PathBuf> {
        let name = safe_file_name(&source.name);
        let path = self.files_dir(id)?.join(format!("{random_id}-{name}"));
        source.rewind()?;
        let parent = path.parent().ok_or(Error::Storage)?;
        crate::temporary::cleanup(parent, ".saved-copy-");
        let mut temporary = crate::temporary::create(parent, ".saved-copy-")?;
        let size = source.size;
        let mut limited = source.reader().take(size);
        let copied = std::io::copy(&mut limited, &mut temporary).map_err(|_| Error::Storage)?;
        if copied != size {
            return Err(Error::Media);
        }
        temporary.as_file().sync_all().map_err(|_| Error::Storage)?;
        temporary.persist(&path).map_err(|_| Error::Storage)?;
        self.completed_file(id, random_id, &path)?;
        Ok(path)
    }
}
pub fn safe_file_name(value: &str) -> String {
    let name = value.rsplit(['/', '\\']).next().unwrap_or("");
    let safe: String = name
        .chars()
        .map(|c| {
            if c < ' ' || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    if safe.is_empty() {
        "file.bin".into()
    } else {
        safe
    }
}
fn restrict_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Storage)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
