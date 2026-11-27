//! Filesystem operations shared by authenticated stores.
//!
//! The caller owns directory protection. A persistent sidecar lock coordinates
//! cooperating writers; it must remain empty and must never be unlinked while a
//! store may be in use. An encrypted data file cannot also act as a lock sidecar.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub(crate) const MAX_STORE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn absolute_store_path(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent = parent.canonicalize()?;
    if !parent.is_dir() {
        return Err(io::Error::from(io::ErrorKind::NotADirectory));
    }
    Ok(parent.join(name))
}

fn regular_file_or_missing(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn restrict_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

pub(crate) fn lock_store(path: &Path) -> io::Result<File> {
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?
        .to_os_string();
    name.push(".lock");
    let lock_path = path.with_file_name(name);
    let file = match private_options().create_new(true).open(&lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !regular_file_or_missing(&lock_path)? {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            private_options().open(&lock_path)?
        }
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    file.try_lock().map_err(io::Error::from)?;
    // Store updates replace their data inode. Using one as a lock would let a
    // cooperating store replace a held lock and admit another independent writer.
    if file.metadata()?.len() != 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    restrict_permissions(&file)?;
    Ok(file)
}

pub(crate) fn read_store(path: &Path) -> io::Result<Option<Vec<u8>>> {
    if !regular_file_or_missing(path)? {
        return Ok(None);
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_STORE_BYTES as u64 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    restrict_permissions(&file)?;
    let mut raw = Vec::new();
    file.take((MAX_STORE_BYTES + 1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_STORE_BYTES {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(Some(raw))
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) struct ReplaceFailure {
    pub(crate) error: io::Error,
    /// Replacement happened, but its durability could not be established.
    pub(crate) replaced: bool,
}

impl From<io::Error> for ReplaceFailure {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            replaced: false,
        }
    }
}

struct StagedFile {
    file: Option<File>,
    path: Option<PathBuf>,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Clone, Copy)]
enum CommitStage {
    BeforeRename,
    AfterRename,
}

pub(crate) fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), ReplaceFailure> {
    replace_with(path, bytes, |_| Ok(()))
}

fn replace_with(
    path: &Path,
    bytes: &[u8],
    checkpoint: impl Fn(CommitStage) -> io::Result<()>,
) -> Result<(), ReplaceFailure> {
    if bytes.len() > MAX_STORE_BYTES {
        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
    }
    regular_file_or_missing(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| io::Error::from(io::ErrorKind::Other))?;
    let staged_path = parent.join(format!(
        ".ksc-store-{}-{}.tmp",
        std::process::id(),
        hex::encode(random)
    ));
    let file = private_options().create_new(true).open(&staged_path)?;
    let mut staged = StagedFile {
        file: Some(file),
        path: Some(staged_path.clone()),
    };
    let file = staged.file.as_mut().expect("staging file was just created");
    file.write_all(bytes)?;
    file.sync_all()?;
    staged.file.take();
    checkpoint(CommitStage::BeforeRename)?;
    fs::rename(&staged_path, path)?;
    staged.path = None;
    checkpoint(CommitStage::AfterRename)
        .and_then(|_| sync_parent(path))
        .map_err(|error| ReplaceFailure {
            error,
            replaced: true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_staging_preserves_previous_file_and_cleans_only_owned_temp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
        fs::write(&path, b"prior state").unwrap();
        let result = replace_with(&path, b"next state", |stage| match stage {
            CommitStage::BeforeRename => Err(io::Error::from(io::ErrorKind::StorageFull)),
            CommitStage::AfterRename => Ok(()),
        });
        assert!(result.is_err_and(|error| !error.replaced));
        assert_eq!(fs::read(&path).unwrap(), b"prior state");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_post_replace_sync_reports_uncertain_commit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
        fs::write(&path, b"prior state").unwrap();
        let result = replace_with(&path, b"next state", |stage| match stage {
            CommitStage::BeforeRename => Ok(()),
            CommitStage::AfterRename => Err(io::Error::from(io::ErrorKind::StorageFull)),
        });
        assert!(result.is_err_and(|error| error.replaced));
        assert_eq!(fs::read(&path).unwrap(), b"next state");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
