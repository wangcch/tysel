//! Official filesystem capability.
//!
//! Trusted-path `tysel.fs.read` / `tysel.fs.write` are confined to configured
//! roots. Paths are opened relative to a directory fd (`openat` / Linux
//! `openat2`) so a symlink swap cannot escape the allowlist. Unconfigured
//! processes deny every path. Isolated workers never call this crate.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

const MAX_BYTES: u64 = 1_048_576;

struct Policy {
    base: PathBuf,
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

static POLICY: RwLock<Option<Policy>> = RwLock::new(None);

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Replace the process-wide filesystem allowlists. Relative roots and relative
/// request paths are resolved against `root` when provided, otherwise the
/// process working directory. An empty list denies that operation.
pub fn configure(read: Vec<String>, write: Vec<String>, root: Option<&Path>) {
    let base = root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    *POLICY.write().expect("fs policy lock") = Some(Policy {
        read: resolve_roots(&read, &base),
        write: resolve_roots(&write, &base),
        base,
    });
}

pub fn read(path: &str) -> Result<String, String> {
    let guard = POLICY.read().expect("fs policy lock");
    read_with(path, guard.as_ref())
}

pub fn write(path: &str, data: &str) -> Result<(), String> {
    let guard = POLICY.read().expect("fs policy lock");
    write_with(path, data, guard.as_ref())
}

fn read_with(path: &str, policy: Option<&Policy>) -> Result<String, String> {
    let policy = policy.ok_or("filesystem is not configured")?;
    let mut file = open_confined(path, &policy.base, &policy.read, false)?;
    let len = file.metadata().map_err(|err| err.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("file exceeds {MAX_BYTES} bytes"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|err| err.to_string())?;
    String::from_utf8(bytes).map_err(|_| "file is not valid utf-8".into())
}

fn write_with(path: &str, data: &str, policy: Option<&Policy>) -> Result<(), String> {
    let policy = policy.ok_or("filesystem is not configured")?;
    if data.len() as u64 > MAX_BYTES {
        return Err(format!("write exceeds {MAX_BYTES} bytes"));
    }
    let mut file = open_confined(path, &policy.base, &policy.write, true)?;
    file.write_all(data.as_bytes()).map_err(|err| err.to_string())
}

fn resolve_roots(paths: &[String], base: &Path) -> Vec<PathBuf> {
    paths
        .iter()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(absolute(Path::new(trimmed), base))
        })
        .collect()
}

fn open_confined(path: &str, base: &Path, roots: &[PathBuf], write: bool) -> Result<File, String> {
    if path.trim().is_empty() {
        return Err("path must not be empty".into());
    }
    if roots.is_empty() {
        return Err("path is not permitted".into());
    }
    let requested = absolute(Path::new(path), base);
    let request_parts = lexical_components(&requested)?;
    for root in roots {
        let root_parts = lexical_components(root)?;
        if let Some(relative) = strip_prefix(&request_parts, &root_parts) {
            if relative.is_empty() {
                return Err("path is not permitted".into());
            }
            return open_beneath(root, &relative, write);
        }
    }
    Err("path is not permitted".into())
}

fn lexical_components(path: &Path) -> Result<Vec<std::ffi::OsString>, String> {
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {}
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return Err("path is not permitted".into()),
            std::path::Component::Normal(name) => {
                if name.is_empty() || name == ".." {
                    return Err("path is not permitted".into());
                }
                out.push(name.to_os_string());
            }
        }
    }
    Ok(out)
}

fn strip_prefix(
    path: &[std::ffi::OsString],
    root: &[std::ffi::OsString],
) -> Option<Vec<std::ffi::OsString>> {
    if path.len() < root.len() {
        return None;
    }
    if path.iter().zip(root.iter()).any(|(left, right)| left != right) {
        return None;
    }
    Some(path[root.len()..].to_vec())
}

fn absolute(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
}

#[cfg(unix)]
fn open_beneath(root: &Path, relative: &[std::ffi::OsString], write: bool) -> Result<File, String> {
    unix::open_beneath(root, relative, write)
}

#[cfg(not(unix))]
fn open_beneath(
    _root: &Path,
    _relative: &[std::ffi::OsString],
    _write: bool,
) -> Result<File, String> {
    Err("filesystem confinement requires unix".into())
}

#[cfg(unix)]
mod unix {
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::File;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    use std::path::Path;

    pub fn open_beneath(root: &Path, relative: &[OsString], write: bool) -> Result<File, String> {
        let dir = open_dir(root)?;
        #[cfg(target_os = "linux")]
        if let Ok(file) = openat2_beneath(&dir, relative, write) {
            return Ok(file);
        }
        openat_walk(dir, relative, write)
    }

    fn open_dir(path: &Path) -> Result<OwnedFd, String> {
        let c_path = cstring(path.as_os_str())?;
        // SAFETY: c_path is a valid C string for the duration of the call.
        #[allow(unsafe_code)]
        let fd = unsafe {
            libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC)
        };
        owned_fd(fd)
    }

    fn openat_walk(root: OwnedFd, relative: &[OsString], write: bool) -> Result<File, String> {
        let mut current = root;
        for (index, component) in relative.iter().enumerate() {
            let last = index + 1 == relative.len();
            let mut flags = libc::O_CLOEXEC | libc::O_NOFOLLOW;
            if last && write {
                flags |= libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC;
            } else if last {
                flags |= libc::O_RDONLY;
            } else {
                flags |= libc::O_RDONLY | libc::O_DIRECTORY;
            }
            let mode = if last && write { 0o644 } else { 0 };
            let c_name = cstring(component)?;
            // SAFETY: current is an open directory fd; c_name is a valid C string.
            #[allow(unsafe_code)]
            let fd = unsafe { libc::openat(current.as_raw_fd(), c_name.as_ptr(), flags, mode) };
            current = owned_fd(fd)?;
        }
        Ok(File::from(current))
    }

    #[cfg(target_os = "linux")]
    fn openat2_beneath(root: &OwnedFd, relative: &[OsString], write: bool) -> Result<File, String> {
        #[repr(C)]
        struct OpenHow {
            flags: u64,
            mode: u64,
            resolve: u64,
        }
        let mut flags = (libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64;
        let mut mode = 0u64;
        if write {
            flags |= (libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC) as u64;
            mode = 0o644;
        } else {
            flags |= libc::O_RDONLY as u64;
        }
        let how = OpenHow {
            flags,
            mode,
            resolve: libc::RESOLVE_BENEATH
                | libc::RESOLVE_NO_SYMLINKS
                | libc::RESOLVE_NO_MAGICLINKS,
        };
        let rel = join_relative(relative)?;
        // SAFETY: root is an open directory fd; rel and how are valid for the call.
        #[allow(unsafe_code)]
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                root.as_raw_fd(),
                rel.as_ptr(),
                std::ptr::from_ref(&how),
                std::mem::size_of::<OpenHow>(),
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: openat2 returned a new file descriptor we now own.
        #[allow(unsafe_code)]
        Ok(File::from(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }

    #[cfg(target_os = "linux")]
    fn join_relative(relative: &[OsString]) -> Result<CString, String> {
        let mut bytes = Vec::new();
        for (index, part) in relative.iter().enumerate() {
            if index > 0 {
                bytes.push(b'/');
            }
            bytes.extend_from_slice(part.as_os_str().as_bytes());
        }
        CString::new(bytes).map_err(|_| "path must not contain NUL".into())
    }

    fn cstring(name: &OsStr) -> Result<CString, String> {
        CString::new(name.as_bytes()).map_err(|_| "path must not contain NUL".into())
    }

    fn owned_fd(fd: libc::c_int) -> Result<OwnedFd, String> {
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: fd is a newly opened descriptor.
        #[allow(unsafe_code)]
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tysel-fs-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    fn policy_for(dir: &Path, read: &[&str], write: &[&str]) -> Policy {
        let read: Vec<String> = read.iter().map(|path| (*path).to_string()).collect();
        let write: Vec<String> = write.iter().map(|path| (*path).to_string()).collect();
        Policy {
            read: resolve_roots(&read, dir),
            write: resolve_roots(&write, dir),
            base: dir.to_path_buf(),
        }
    }

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn unconfigured_denies_reads() {
        let err = read_with("data/hello.txt", None).unwrap_err();
        assert!(err.contains("not configured"), "{err}");
    }

    #[test]
    fn empty_allowlist_denies() {
        let dir = temp_tree("empty");
        let policy = policy_for(&dir, &[], &[]);
        let err = read_with("data/hello.txt", Some(&policy)).unwrap_err();
        assert!(err.contains("not permitted"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_and_write_stay_inside_roots() {
        let dir = temp_tree("ok");
        std::fs::write(dir.join("data/hello.txt"), "hi").unwrap();
        let policy = policy_for(&dir, &["./data"], &["./data"]);
        assert_eq!(read_with("data/hello.txt", Some(&policy)).unwrap(), "hi");
        write_with("data/out.txt", "ok", Some(&policy)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("data/out.txt")).unwrap(), "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn traversal_cannot_escape_root() {
        let dir = temp_tree("escape");
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        std::fs::write(dir.join("data/hello.txt"), "hi").unwrap();
        let policy = policy_for(&dir, &["./data"], &["./data"]);
        let err = read_with("data/../secret.txt", Some(&policy)).unwrap_err();
        assert!(err.contains("not permitted"), "{err}");
        let err = write_with("data/../secret.txt", "x", Some(&policy)).unwrap_err();
        assert!(err.contains("not permitted"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("secret.txt")).unwrap(), "no");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cannot_escape_root() {
        let dir = temp_tree("symlink");
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        std::os::unix::fs::symlink(dir.join("secret.txt"), dir.join("data/link")).unwrap();
        let policy = policy_for(&dir, &["./data"], &["./data"]);
        let err = read_with("data/link", Some(&policy)).unwrap_err();
        assert!(!err.contains("no\n") && !err.contains("not configured"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("secret.txt")).unwrap(), "no");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_oversized_writes() {
        let dir = temp_tree("size");
        let policy = policy_for(&dir, &[], &["./data"]);
        let data = "a".repeat((MAX_BYTES as usize) + 1);
        let err = write_with("data/big.txt", &data, Some(&policy)).unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
