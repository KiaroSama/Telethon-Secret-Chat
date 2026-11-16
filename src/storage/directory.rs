use crate::error::{Error, Result};
use std::{fs, path::Path};

#[cfg(all(test, unix))]
thread_local! {
    pub(super) static SYNCED_PARENTS: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    pub(super) static FAIL_NEXT_PARENT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn create_parent(parent: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut current = parent;
    while !current.exists() {
        missing.push(current.to_path_buf());
        current = current.parent().ok_or(Error::Storage)?;
    }
    if !missing.is_empty() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|_| Error::Storage)?;
    }
    // Existence does not prove that an earlier failed initialization synced
    // these entries. Retry the containing directories even when creation is
    // unnecessary, working from the filesystem root toward the store.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let store_device = fs::metadata(parent).map_err(|_| Error::Storage)?.dev();
        let mut containing_directories = Vec::new();
        for containing in parent.ancestors().skip(1) {
            // A mount's parent is not modified when descendants on this
            // filesystem are created (notably the read-only macOS system volume).
            if fs::metadata(containing).map_err(|_| Error::Storage)?.dev() != store_device {
                break;
            }
            containing_directories.push(containing);
        }
        for containing in containing_directories.into_iter().rev() {
            #[cfg(test)]
            if FAIL_NEXT_PARENT.with(|path| {
                let mut path = path.borrow_mut();
                if path.as_deref() == Some(containing) {
                    path.take();
                    true
                } else {
                    false
                }
            }) {
                return Err(Error::Storage);
            }
            fs::File::open(containing)
                .and_then(|file| file.sync_all())
                .map_err(|_| Error::Storage)?;
            #[cfg(test)]
            SYNCED_PARENTS.with(|paths| paths.borrow_mut().push(containing.to_path_buf()));
        }
    }
    Ok(())
}
