use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

struct PendingFile(PathBuf);

impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// Create the temporary file exclusively beside its destination. Renaming an
// existing file replaces it atomically on the supported Unix and Windows hosts.
// Never unlink the old manifest or reuse a caller-owned fixed `.tmp` path.
pub fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|_| io::Error::other("temporary file entropy unavailable"))?;
    let temporary = parent.join(format!(".schema-{}.tmp", hex::encode(nonce)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let pending = PendingFile(temporary);
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&pending.0, path)
}
