use crate::error::{Error, Result};
use std::{fs, path::Path};

#[cfg(all(test, unix))]
thread_local! {
    pub(super) static SYNCED_PARENTS: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(super) fn create_parent(parent: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut current = parent;
    while !current.exists() {
        missing.push(current.to_path_buf());
        current = current.parent().ok_or(Error::Storage)?;
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent).map_err(|_| Error::Storage)?;
    #[cfg(unix)]
    for created in missing.iter().rev() {
        let containing = created.parent().ok_or(Error::Storage)?;
        fs::File::open(containing)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Storage)?;
        #[cfg(test)]
        SYNCED_PARENTS.with(|paths| paths.borrow_mut().push(containing.to_path_buf()));
    }
    Ok(())
}
