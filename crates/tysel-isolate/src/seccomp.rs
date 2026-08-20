//! Linux seccomp filter for isolated workers.
//!
//! This first slice is a denylist: exec, ptrace, mount, modules, and a few
//! other kernel-attack syscalls return EPERM. It is not a full allowlist.
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
    use std::mem::offset_of;

    use super::IsolateError;

    const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn denied_syscalls() -> &'static [u32] {
        &[
            libc::SYS_execve as u32,
            libc::SYS_execveat as u32,
            libc::SYS_ptrace as u32,
            libc::SYS_mount as u32,
            libc::SYS_umount2 as u32,
            libc::SYS_pivot_root as u32,
            libc::SYS_swapon as u32,
            libc::SYS_swapoff as u32,
            libc::SYS_init_module as u32,
            libc::SYS_finit_module as u32,
            libc::SYS_delete_module as u32,
            libc::SYS_bpf as u32,
            libc::SYS_userfaultfd as u32,
            libc::SYS_perf_event_open as u32,
            libc::SYS_kexec_load as u32,
            libc::SYS_kexec_file_load as u32,
            libc::SYS_reboot as u32,
            libc::SYS_unshare as u32,
            libc::SYS_setns as u32,
            libc::SYS_capset as u32,
            libc::SYS_open_by_handle_at as u32,
            libc::SYS_process_vm_readv as u32,
            libc::SYS_process_vm_writev as u32,
            libc::SYS_fsopen as u32,
            libc::SYS_fsmount as u32,
            libc::SYS_move_mount as u32,
            libc::SYS_mount_setattr as u32,
        ]
    }

    pub fn restrict() -> Result<(), IsolateError> {
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            return Err(IsolateError::Limit(
                "seccomp is only implemented for x86_64 and aarch64".into(),
            ));
        }
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            install()
        }
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn install() -> Result<(), IsolateError> {
        // SAFETY: PR_SET_NO_NEW_PRIVS is required before seccomp filters.
        #[allow(unsafe_code)]
        let nnp = unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if nnp != 0 {
            return Err(os_err("prctl(PR_SET_NO_NEW_PRIVS)"));
        }

        let mut filters = program();
        let prog = libc::sock_fprog {
            len: u16::try_from(filters.len())
                .map_err(|_| IsolateError::Limit("seccomp filter is too large".into()))?,
            filter: filters.as_mut_ptr(),
        };
        // SAFETY: seccomp copies the filter; TSYNC applies it to every thread.
        #[allow(unsafe_code)]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_TSYNC,
                std::ptr::from_ref(&prog),
            )
        };
        if rc < 0 {
            return Err(os_err("seccomp"));
        }
        Ok(())
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn program() -> Vec<libc::sock_filter> {
        let ld_abs = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
        let jmp_eq = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
        let ret = (libc::BPF_RET | libc::BPF_K) as u16;
        let mut filters = vec![
            stmt(ld_abs, offset_of!(libc::seccomp_data, arch) as u32),
            jump(jmp_eq, AUDIT_ARCH, 1, 0),
            stmt(ret, libc::SECCOMP_RET_KILL_PROCESS),
            stmt(ld_abs, offset_of!(libc::seccomp_data, nr) as u32),
        ];
        for nr in denied_syscalls() {
            filters.push(jump(jmp_eq, *nr, 0, 1));
            filters.push(stmt(ret, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32));
        }
        filters.push(stmt(ret, libc::SECCOMP_RET_ALLOW));
        filters
    }

    fn stmt(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt: 0, jf: 0, k }
    }

    fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
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
            apply().expect("seccomp no-op");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn seccomp_denies_execve() {
        // SAFETY: the child only applies seccomp then exits; the parent waits.
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert_ne!(pid, -1, "fork");
        if pid == 0 {
            let denied = apply().is_ok() && execve_is_denied();
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
            "seccomp probe exit {status}"
        );
    }

    #[cfg(target_os = "linux")]
    fn execve_is_denied() -> bool {
        let path = c"/tysel-seccomp-should-not-exist";
        let argv = [path.as_ptr(), std::ptr::null()];
        let envp = [std::ptr::null::<libc::c_char>()];
        // SAFETY: pointers are valid C strings for the duration of the call.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
        rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
