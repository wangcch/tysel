//! Best-effort cgroup v2 memory.max for isolated workers.
//!
//! The supervisor attaches the worker pid after spawn. Creating a cgroup often
//! fails in CI and on developer machines (EACCES / ENOENT / EROFS); those
//! failures are skipped rather than failing isolate start. macOS is a no-op.

use std::fs;
#[cfg(any(test, target_os = "linux"))]
use std::io::{self, ErrorKind};
#[cfg(any(test, target_os = "linux"))]
use std::path::Path;
use std::path::PathBuf;

pub struct Guard {
    path: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if fs::remove_dir(&self.path).is_err() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(any(test, target_os = "linux"))]
pub fn dir_name(pid: u32) -> String {
    format!("tysel-worker-{pid}")
}

/// Attach `pid` to a memory cgroup capped at `memory_max_bytes`. Returns
/// `None` when the host cannot create or write the cgroup.
pub fn attach(pid: u32, memory_max_bytes: usize) -> Option<Guard> {
    #[cfg(target_os = "linux")]
    {
        linux::attach(pid, memory_max_bytes)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, memory_max_bytes);
        None
    }
}

#[cfg(any(test, target_os = "linux"))]
pub fn attach_to(base: &Path, pid: u32, memory_max_bytes: usize) -> io::Result<Guard> {
    let dir = base.join(dir_name(pid));
    fs::create_dir(&dir)?;
    if let Err(err) = fs::write(dir.join("memory.max"), memory_max_bytes.to_string()) {
        let _ = fs::remove_dir_all(&dir);
        return Err(err);
    }
    if let Err(err) = fs::write(dir.join("cgroup.procs"), pid.to_string()) {
        let _ = fs::remove_dir_all(&dir);
        return Err(err);
    }
    Ok(Guard { path: dir })
}

#[cfg(any(test, target_os = "linux"))]
fn skippable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::PermissionDenied
            | ErrorKind::NotFound
            | ErrorKind::ReadOnlyFilesystem
            | ErrorKind::AlreadyExists
            | ErrorKind::InvalidInput
            | ErrorKind::Unsupported
    )
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;

    use super::{Guard, attach_to, skippable};

    pub fn attach(pid: u32, memory_max_bytes: usize) -> Option<Guard> {
        let base = std::env::var_os("TYSEL_CGROUP")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/sys/fs/cgroup"));
        match attach_to(&base, pid, memory_max_bytes) {
            Ok(guard) => Some(guard),
            Err(err) if skippable(&err) => None,
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_name_includes_pid() {
        assert_eq!(dir_name(42), "tysel-worker-42");
    }

    #[test]
    fn attach_to_writes_memory_max_and_procs() {
        let dir = std::env::temp_dir().join(format!(
            "tysel-cgroup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let guard = attach_to(&dir, 99, 256 * 1024 * 1024).unwrap();
        let child = dir.join("tysel-worker-99");
        assert_eq!(
            fs::read_to_string(child.join("memory.max")).unwrap().trim(),
            &(256 * 1024 * 1024).to_string()
        );
        assert_eq!(fs::read_to_string(child.join("cgroup.procs")).unwrap().trim(), "99");
        drop(guard);
        assert!(!child.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skippable_covers_permission_and_missing() {
        assert!(skippable(&io::Error::from(ErrorKind::PermissionDenied)));
        assert!(skippable(&io::Error::from(ErrorKind::NotFound)));
        assert!(skippable(&io::Error::from(ErrorKind::ReadOnlyFilesystem)));
        assert!(!skippable(&io::Error::from(ErrorKind::BrokenPipe)));
    }
}
