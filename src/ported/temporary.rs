//! Track temporary-file owners and reap only proven inactive entries.

use std::{
    collections::HashSet,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

type Outcome<T> = Result<T, String>;
static ACTIVE: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(0);

fn valid_prefix(prefix: &str) -> Outcome<()> {
    if !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        Ok(())
    } else {
        Err("ValueError: expected a nonempty ASCII fixture prefix".into())
    }
}

fn owner(name: &str, prefix: &str) -> Option<u32> {
    let (pid, tail) = name
        .strip_prefix(prefix)?
        .strip_suffix(".tmp")?
        .split_once('-')?;
    if pid.starts_with('0')
        || !pid.bytes().all(|b| b.is_ascii_digit())
        || tail.is_empty()
        || !tail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    pid.parse().ok()
}

#[cfg(windows)]
fn proven_dead(pid: u32) -> bool {
    let Some(system) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let query = PathBuf::from(system).join("System32/tasklist.exe");
    process::Command::new(query)
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(process::Stdio::null())
        .output()
        .is_ok_and(|out| {
            out.status.success()
                && out.stderr.is_empty()
                && std::str::from_utf8(&out.stdout).is_ok_and(|s| {
                    s.trim() == "INFO: No tasks are running which match the specified criteria."
                })
        })
}

#[cfg(not(windows))]
fn proven_dead(_pid: u32) -> bool {
    false
}

fn cleanup(directory: &Path, prefix: &str, active: &HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|name| owner(name, prefix)) else {
            continue;
        };
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_file())
            || active.contains(&path)
            || (pid != process::id() && !proven_dead(pid))
        {
            continue;
        }
        let _ = fs::remove_file(path);
    }
}

fn io_error(error: io::Error) -> String {
    format!("OSError: {:?}", error.kind())
}

fn scratch() -> Outcome<PathBuf> {
    let root = std::env::temp_dir();
    for _ in 0..20 {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("tsc-oracle-{}-{id}", process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return path.canonicalize().map_err(io_error),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(io_error(e)),
        }
    }
    Err("OSError: could not create fixture directory".into())
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn owned_roundtrip(prefix: String) -> Outcome<Vec<bool>> {
    valid_prefix(&prefix)?;
    let path = Scratch(scratch()?);
    let mut guard = ACTIVE
        .lock()
        .map_err(|_| "RuntimeError: cannot lock temporary ownership")?;
    let active = guard.get_or_insert_with(HashSet::new);
    cleanup(&path.0, &prefix, active);
    let current = path.0.join(format!("{prefix}{}-active.tmp", process::id()));
    let file = File::create_new(&current).map_err(io_error)?;
    active.insert(current.clone());
    cleanup(&path.0, &prefix, active);
    let while_active = current.is_file();
    drop(file);
    active.remove(&current);
    cleanup(&path.0, &prefix, active);
    Ok(vec![while_active, current.exists()])
}

pub fn cleanup_fixture(prefix: String, owners: Vec<String>) -> Outcome<Vec<bool>> {
    valid_prefix(&prefix)?;
    let path = Scratch(scratch()?);
    let paths = owners
        .iter()
        .map(|owner| {
            path.0.join(format!(
                "{prefix}{}.tmp",
                owner.replace("{self}", &process::id().to_string())
            ))
        })
        .collect::<Vec<_>>();
    for item in &paths {
        if item.parent() == Some(path.0.as_path()) {
            fs::write(item, b"fixture").map_err(io_error)?;
        }
    }
    cleanup(&path.0, &prefix, &HashSet::new());
    Ok(paths
        .iter()
        .map(|item| item.parent() == Some(path.0.as_path()) && item.exists())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_syntax_rejects_unproven_names() {
        assert_eq!(owner("x-194-own.tmp", "x-"), Some(194));
        assert_eq!(owner("x-0194-own.tmp", "x-"), None);
        assert_eq!(owner("x-194-OWN.tmp", "x-"), None);
        assert_eq!(owner("x-4294967296-own.tmp", "x-"), None);
    }
    #[test]
    fn cleanup_only_reaps_after_release() {
        assert_eq!(owned_roundtrip(".fixture-".into()), Ok(vec![true, false]));
    }
}
