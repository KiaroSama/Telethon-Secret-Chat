use crate::error::{Error, Result};
use std::{fs, path::Path};

#[cfg(all(test, unix))]
thread_local! {
    pub(super) static SYNCED_PARENTS: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    pub(super) static FAIL_NEXT_PARENT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn create_parent(parent: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        // Recover known creation intents using name lookup, which requires
        // search permission rather than read access to existing ancestors.
        let mut lineage = parent.ancestors().collect::<Vec<_>>();
        lineage.reverse();
        for entry in lineage {
            let missing = match fs::metadata(entry) {
                Ok(metadata) if metadata.is_dir() => false,
                Ok(_) => return Err(Error::Storage),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => return Err(Error::Storage),
            };
            settle_entry(entry, missing)?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent).map_err(|_| Error::Storage)
}

#[cfg(unix)]
fn sync_directory(parent: &Path, publication: bool) -> Result<()> {
    #[cfg(test)]
    if publication
        && FAIL_NEXT_PARENT.with(|path| {
            let mut path = path.borrow_mut();
            if path.as_deref() == Some(parent) {
                path.take();
                true
            } else {
                false
            }
        })
    {
        return Err(Error::Storage);
    }
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::Storage)?;
    #[cfg(test)]
    if publication {
        SYNCED_PARENTS.with(|paths| paths.borrow_mut().push(parent.to_path_buf()));
    }
    #[cfg(not(test))]
    let _ = publication;
    Ok(())
}

#[cfg(unix)]
fn settle_entry(entry: &Path, missing: bool) -> Result<()> {
    use sha2::Digest;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    };

    let (Some(parent), Some(name)) = (entry.parent(), entry.file_name()) else {
        return if missing { Err(Error::Storage) } else { Ok(()) };
    };
    let digest = sha2::Sha256::digest(name.as_bytes());
    let intent = parent.join(format!(".kiaro-mkdir-{}.pending", hex::encode(digest)));
    if !missing {
        match fs::symlink_metadata(&intent) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Error::Storage),
        }
    }
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let record = match options.create_new(true).open(&intent) {
        Ok(record) => record,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => options
            .create_new(false)
            .open(&intent)
            .map_err(|_| Error::Storage)?,
        Err(_) => return Err(Error::Storage),
    };
    let metadata = record.metadata().map_err(|_| Error::Storage)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() != 0
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(Error::Storage);
    }
    record.try_lock().map_err(|_| Error::Storage)?;
    let result = (|| {
        // The intent must precede mkdir durably. A process can die between any
        // two calls, and a retry must recognize an existing but unsynced entry.
        record.sync_all().map_err(|_| Error::Storage)?;
        sync_directory(parent, false)?;
        if missing {
            match fs::DirBuilder::new().mode(0o700).create(entry) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && fs::metadata(entry).is_ok_and(|metadata| metadata.is_dir()) => {}
                Err(_) => return Err(Error::Storage),
            }
        }
        sync_directory(parent, true)?;
        fs::remove_file(&intent).map_err(|_| Error::Storage)?;
        sync_directory(parent, false)
    })();
    let unlocked = record.unlock().map_err(|_| Error::Storage);
    result.and(unlocked)
}
