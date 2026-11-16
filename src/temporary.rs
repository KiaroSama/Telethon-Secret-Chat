//! Ownership-aware temporary files. Cleanup requires affirmative owner-death evidence.
use crate::error::{Error, Result};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
static ACTIVE: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
pub(crate) struct OwnedTemporary {
    file: Option<tempfile::NamedTempFile>,
    path: PathBuf,
}
fn prefix_valid(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.len() <= 128
        && prefix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
}
pub(crate) fn create(parent: &Path, prefix: &str) -> Result<OwnedTemporary> {
    if !prefix_valid(prefix) {
        return Err(Error::Storage);
    }
    let parent = std::path::absolute(parent).map_err(|_| Error::Storage)?;
    let mut active = ACTIVE.lock().map_err(|_| Error::Storage)?;
    let file = tempfile::Builder::new()
        .prefix(&format!("{prefix}{}-", std::process::id()))
        .rand_bytes(12)
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| Error::Storage)?;
    file.as_file().try_lock().map_err(|_| Error::Storage)?;
    let path = file.path().to_path_buf();
    active.insert(path.clone());
    Ok(OwnedTemporary {
        file: Some(file),
        path,
    })
}
impl OwnedTemporary {
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn as_file(&self) -> &File {
        match &self.file {
            Some(file) => file.as_file(),
            None => unreachable!("persist consumes temporary ownership"),
        }
    }
    pub(crate) fn as_file_mut(&mut self) -> &mut File {
        match &mut self.file {
            Some(file) => file.as_file_mut(),
            None => unreachable!("persist consumes temporary ownership"),
        }
    }
    pub(crate) fn persist(mut self, destination: &Path) -> Result<File> {
        self.file
            .take()
            .ok_or(Error::Storage)?
            .persist(destination)
            .map_err(|_| Error::Storage)
    }
}
impl Drop for OwnedTemporary {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.path);
        }
    }
}
impl Deref for OwnedTemporary {
    type Target = File;
    fn deref(&self) -> &File {
        self.as_file()
    }
}
impl DerefMut for OwnedTemporary {
    fn deref_mut(&mut self) -> &mut File {
        self.as_file_mut()
    }
}
impl Read for OwnedTemporary {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.as_file_mut().read(buffer)
    }
}
impl Write for OwnedTemporary {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.as_file_mut().write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.as_file_mut().flush()
    }
}
impl Seek for OwnedTemporary {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.as_file_mut().seek(position)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessState {
    Alive,
    Dead,
    Unknown,
}
#[cfg(any(target_os = "linux", test))]
fn process_lookup(result: io::Result<()>) -> ProcessState {
    match result {
        Ok(()) => ProcessState::Alive,
        Err(error) if error.kind() == io::ErrorKind::NotFound => ProcessState::Dead,
        Err(_) => ProcessState::Unknown,
    }
}
fn native_process(pid: u32) -> ProcessState {
    if pid == 0 || pid > i32::MAX as u32 {
        return ProcessState::Unknown;
    }
    #[cfg(target_os = "linux")]
    {
        process_lookup(std::fs::metadata(Path::new("/proc").join(pid.to_string())).map(|_| ()))
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .output();
        match output {
            Ok(output)
                if output.status.success()
                    && std::str::from_utf8(&output.stdout)
                        .ok()
                        .is_some_and(|text| {
                            text.split_whitespace()
                                .any(|part| part.parse::<u32>() == Ok(pid))
                        }) =>
            {
                ProcessState::Alive
            }
            Ok(output)
                if output.status.code() == Some(1)
                    && output.stdout.is_empty()
                    && output.stderr.is_empty() =>
            {
                ProcessState::Dead
            }
            _ => ProcessState::Unknown,
        }
    }
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let Ok(text) = std::str::from_utf8(&output.stdout) else {
                    return ProcessState::Unknown;
                };
                if text.lines().any(|line| {
                    line.split(',')
                        .nth(1)
                        .is_some_and(|value| value.trim_matches('"').parse::<u32>() == Ok(pid))
                }) {
                    ProcessState::Alive
                } else if !text.contains('"') && !text.trim().is_empty() && output.stderr.is_empty()
                {
                    ProcessState::Dead
                } else {
                    ProcessState::Unknown
                }
            }
            _ => ProcessState::Unknown,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        ProcessState::Unknown
    }
}
pub(crate) fn cleanup(parent: &Path, prefix: &str) {
    cleanup_with(parent, prefix, native_process, |path| {
        std::fs::remove_file(path)
    });
}
fn cleanup_with(
    parent: &Path,
    prefix: &str,
    probe: impl Fn(u32) -> ProcessState,
    remove: impl Fn(&Path) -> io::Result<()>,
) {
    if !prefix_valid(prefix) {
        return;
    }
    let Ok(parent) = std::path::absolute(parent) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        tracing::warn!("Owned temporary cleanup deferred");
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some((pid, suffix)) = rest.split_once('-') else {
            continue;
        };
        if !suffix.ends_with(".tmp") || suffix.len() <= 4 {
            continue;
        }
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid == 0 || pid > i32::MAX as u32 {
            continue;
        }
        if pid == std::process::id() {
            if !ACTIVE.lock().is_ok_and(|active| !active.contains(&path)) {
                continue;
            }
        } else if probe(pid) != ProcessState::Dead {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        if remove(&path).is_err() {
            tracing::warn!("Owned temporary cleanup deferred");
        }
    }
}
#[cfg(test)]
mod tests;
