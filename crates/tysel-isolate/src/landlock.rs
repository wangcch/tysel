//! Linux Landlock filesystem sandbox for isolated workers.
//!
//! Untrusted QuickJS must not open host files even if the engine is exploited.
//! macOS is not the security gate of record; this is a no-op there.

use crate::supervisor::IsolateError;

pub fn apply() -> Result<(), IsolateError> {
    #[cfg(target_os = "linux")]
    {
        linux::restrict()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::File;
    use std::os::fd::AsRawFd;

    use super::IsolateError;

    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
    const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
    const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

    const ACCESS_EXECUTE: u64 = 1 << 0;
    const ACCESS_WRITE_FILE: u64 = 1 << 1;
    const ACCESS_READ_FILE: u64 = 1 << 2;
    const ACCESS_READ_DIR: u64 = 1 << 3;
    const ACCESS_REMOVE_DIR: u64 = 1 << 4;
    const ACCESS_REMOVE_FILE: u64 = 1 << 5;
    const ACCESS_MAKE_CHAR: u64 = 1 << 6;
    const ACCESS_MAKE_DIR: u64 = 1 << 7;
    const ACCESS_MAKE_REG: u64 = 1 << 8;
    const ACCESS_MAKE_SOCK: u64 = 1 << 9;
    const ACCESS_MAKE_FIFO: u64 = 1 << 10;
    const ACCESS_MAKE_BLOCK: u64 = 1 << 11;
    const ACCESS_MAKE_SYM: u64 = 1 << 12;
    const ACCESS_REFER: u64 = 1 << 13;
    const ACCESS_TRUNCATE: u64 = 1 << 14;
    const ACCESS_IOCTL_DEV: u64 = 1 << 15;

    const ACCESS_FS_ABI1: u64 = ACCESS_EXECUTE
        | ACCESS_WRITE_FILE
        | ACCESS_READ_FILE
        | ACCESS_READ_DIR
        | ACCESS_REMOVE_DIR
        | ACCESS_REMOVE_FILE
        | ACCESS_MAKE_CHAR
        | ACCESS_MAKE_DIR
        | ACCESS_MAKE_REG
        | ACCESS_MAKE_SOCK
        | ACCESS_MAKE_FIFO
        | ACCESS_MAKE_BLOCK
        | ACCESS_MAKE_SYM;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    pub fn restrict() -> Result<(), IsolateError> {
        let abi = abi_version()?;
        let attr = RulesetAttr { handled_access_fs: handled_fs(abi) };
        // SAFETY: landlock_create_ruleset copies the attr and returns a new fd.
        #[allow(unsafe_code)]
        let ruleset = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::from_ref(&attr),
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if ruleset < 0 {
            return Err(os_err("landlock_create_ruleset"));
        }
        let ruleset = ruleset as i32;
        allow_read(ruleset, "/dev/urandom");
        allow_read(ruleset, "/dev/random");
        // SAFETY: PR_SET_NO_NEW_PRIVS is required before restrict_self.
        #[allow(unsafe_code)]
        let nnp = unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if nnp != 0 {
            // SAFETY: close the unused ruleset fd.
            #[allow(unsafe_code)]
            unsafe {
                libc::close(ruleset);
            }
            return Err(os_err("prctl(PR_SET_NO_NEW_PRIVS)"));
        }
        // SAFETY: restrict_self applies the ruleset to this thread and descendants.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) };
        // SAFETY: the kernel duplicates the ruleset; this fd is no longer needed.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(ruleset);
        }
        if rc < 0 {
            return Err(os_err("landlock_restrict_self"));
        }
        Ok(())
    }

    fn abi_version() -> Result<i64, IsolateError> {
        // SAFETY: version probe is a read-only syscall with a null attr.
        #[allow(unsafe_code)]
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        if abi < 1 {
            return Err(os_err("landlock is required on Linux"));
        }
        Ok(abi)
    }

    fn handled_fs(abi: i64) -> u64 {
        let mut access = ACCESS_FS_ABI1;
        if abi >= 2 {
            access |= ACCESS_REFER;
        }
        if abi >= 3 {
            access |= ACCESS_TRUNCATE;
        }
        if abi >= 5 {
            access |= ACCESS_IOCTL_DEV;
        }
        access
    }

    fn allow_read(ruleset: i32, path: &str) {
        let Ok(file) = File::open(path) else {
            return;
        };
        let attr =
            PathBeneathAttr { allowed_access: ACCESS_READ_FILE, parent_fd: file.as_raw_fd() };
        // SAFETY: landlock_add_rule copies attr; the fd must stay open for the call.
        #[allow(unsafe_code)]
        let _ = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset,
                LANDLOCK_RULE_PATH_BENEATH,
                std::ptr::from_ref(&attr),
                0u32,
            )
        };
    }

    fn os_err(op: &str) -> IsolateError {
        IsolateError::Limit(format!("{op}: {}", std::io::Error::last_os_error()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_a_noop_off_linux() {
        if cfg!(not(target_os = "linux")) {
            apply().expect("landlock no-op");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_denies_opening_host_files() {
        // SAFETY: the child only applies Landlock then exits; the parent waits.
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert_ne!(pid, -1, "fork");
        if pid == 0 {
            let denied = apply().is_ok() && std::fs::File::open("/etc/passwd").is_err();
            // SAFETY: the child must not run the rest of the test harness.
            #[allow(unsafe_code)]
            unsafe {
                libc::_exit(if denied { 0 } else { 1 });
            }
        }
        let mut status = 0;
        // SAFETY: wait for the forked probe.
        #[allow(unsafe_code)]
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        assert_eq!(waited, pid);
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "landlock probe exit {status}"
        );
    }
}
