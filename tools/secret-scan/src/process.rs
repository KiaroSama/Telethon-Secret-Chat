//! Private process capture and bounded child groups for the history scanner.
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub type Result<T> = std::result::Result<T, &'static str>;
pub struct PrivateDir(pub PathBuf);
impl PrivateDir {
    pub fn new(parent: &Path) -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "The scanner clock is unavailable.")?
            .as_nanos();
        for attempt in 0..100 {
            let path = parent.join(format!(
                "kiaro-secret-scan-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("The private scanner directory could not be created."),
            }
        }
        Err("The private scanner directory could not be reserved.")
    }
}
impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
}
// Reap the whole private process group on every refusal path, including an
// observation error. A completed child is removed before this guard is dropped.
struct Running(Option<Child>);
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            #[cfg(unix)]
            {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Captured {
    pub fn empty(&self) -> bool {
        [&self.stdout, &self.stderr]
            .iter()
            .all(|p| fs::metadata(p).is_ok_and(|m| m.len() == 0))
    }
}
pub fn run(
    command: &mut Command,
    directory: &Path,
    label: &str,
    timeout: Duration,
    limit: u64,
) -> Result<Captured> {
    let stdout = directory.join(format!("{label}.stdout"));
    let stderr = directory.join(format!("{label}.stderr"));
    command
        .stdout(File::create(&stdout).map_err(|_| "Private process output could not be opened.")?)
        .stderr(File::create(&stderr).map_err(|_| "Private process output could not be opened.")?)
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|_| "A required scanner process could not be started.")?;
    let mut running = Running(Some(child));
    let until = Instant::now() + timeout;
    loop {
        let completed = running
            .0
            .as_mut()
            .ok_or("The scanner process guard is unavailable.")?
            .try_wait()
            .map_err(|_| "A scanner process could not be observed.")?;
        let too_large = [&stdout, &stderr]
            .iter()
            .any(|p| fs::metadata(p).is_ok_and(|m| m.len() > limit));
        if too_large {
            return Err("A scanner process exceeded its bounded deadline or output limit.");
        }
        if let Some(status) = completed {
            running.0 = None;
            return Ok(Captured {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= until {
            return Err("A scanner process exceeded its bounded deadline or output limit.");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
#[cfg(unix)]
pub fn executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| "The verified executable permissions could not be set.")
}
#[cfg(not(unix))]
pub fn executable(_: &Path) -> Result<()> {
    Ok(())
}
pub fn real_git() -> Result<PathBuf> {
    let paths = std::env::var_os("PATH").ok_or("The Git executable is unavailable.")?;
    for directory in std::env::split_paths(&paths) {
        let path = directory.join(if cfg!(windows) { "git.exe" } else { "git" });
        if path.is_file() {
            return fs::canonicalize(path).map_err(|_| "The Git executable could not be resolved.");
        }
    }
    Err("The Git executable is unavailable.")
}
