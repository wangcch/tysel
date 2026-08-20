//! Linux seccomp filter for isolated workers.
//!
//! Isolated workers are restricted to an allowlist: unmatched syscalls return
//! EPERM, and a mismatched architecture kills the process. The list is enough
//! for QuickJS, Rust threads, and the shared Tokio I/O runtime, and it omits
//! sockets, exec, ptrace, mount, and bpf. macOS is not the security gate of
//! record; this is a no-op there.

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
    fn allowed_syscalls() -> Vec<u32> {
        let mut nrs = vec![
            libc::SYS_read as u32,
            libc::SYS_write as u32,
            libc::SYS_close as u32,
            libc::SYS_lseek as u32,
            libc::SYS_readv as u32,
            libc::SYS_writev as u32,
            libc::SYS_pread64 as u32,
            libc::SYS_pwrite64 as u32,
            libc::SYS_openat as u32,
            libc::SYS_newfstatat as u32,
            libc::SYS_fstat as u32,
            libc::SYS_statx as u32,
            libc::SYS_readlinkat as u32,
            libc::SYS_getdents64 as u32,
            libc::SYS_fcntl as u32,
            libc::SYS_ioctl as u32,
            libc::SYS_dup as u32,
            libc::SYS_dup3 as u32,
            libc::SYS_pipe2 as u32,
            libc::SYS_mmap as u32,
            libc::SYS_mprotect as u32,
            libc::SYS_munmap as u32,
            libc::SYS_brk as u32,
            libc::SYS_madvise as u32,
            libc::SYS_mremap as u32,
            libc::SYS_mincore as u32,
            libc::SYS_clone as u32,
            libc::SYS_clone3 as u32,
            libc::SYS_futex as u32,
            libc::SYS_set_robust_list as u32,
            libc::SYS_get_robust_list as u32,
            libc::SYS_set_tid_address as u32,
            libc::SYS_rseq as u32,
            libc::SYS_exit as u32,
            libc::SYS_exit_group as u32,
            libc::SYS_wait4 as u32,
            libc::SYS_waitid as u32,
            libc::SYS_getpid as u32,
            libc::SYS_gettid as u32,
            libc::SYS_tgkill as u32,
            libc::SYS_sched_yield as u32,
            libc::SYS_sched_getaffinity as u32,
            libc::SYS_nanosleep as u32,
            libc::SYS_clock_nanosleep as u32,
            libc::SYS_clock_gettime as u32,
            libc::SYS_clock_getres as u32,
            libc::SYS_gettimeofday as u32,
            libc::SYS_rt_sigaction as u32,
            libc::SYS_rt_sigprocmask as u32,
            libc::SYS_rt_sigreturn as u32,
            libc::SYS_rt_sigtimedwait as u32,
            libc::SYS_sigaltstack as u32,
            libc::SYS_epoll_create1 as u32,
            libc::SYS_epoll_ctl as u32,
            libc::SYS_epoll_pwait as u32,
            libc::SYS_eventfd2 as u32,
            libc::SYS_ppoll as u32,
            libc::SYS_getrandom as u32,
            libc::SYS_getuid as u32,
            libc::SYS_geteuid as u32,
            libc::SYS_getgid as u32,
            libc::SYS_getegid as u32,
            libc::SYS_getrusage as u32,
            libc::SYS_prlimit64 as u32,
            libc::SYS_uname as u32,
            libc::SYS_sysinfo as u32,
            libc::SYS_prctl as u32,
            libc::SYS_membarrier as u32,
            libc::SYS_restart_syscall as u32,
            libc::SYS_faccessat as u32,
            libc::SYS_faccessat2 as u32,
            libc::SYS_timerfd_create as u32,
            libc::SYS_timerfd_settime as u32,
            libc::SYS_timerfd_gettime as u32,
            libc::SYS_fsync as u32,
            libc::SYS_fdatasync as u32,
        ];
        #[cfg(target_arch = "x86_64")]
        {
            nrs.extend([
                libc::SYS_open as u32,
                libc::SYS_stat as u32,
                libc::SYS_lstat as u32,
                libc::SYS_access as u32,
                libc::SYS_pipe as u32,
                libc::SYS_dup2 as u32,
                libc::SYS_poll as u32,
                libc::SYS_select as u32,
                libc::SYS_epoll_wait as u32,
                libc::SYS_readlink as u32,
                libc::SYS_arch_prctl as u32,
                libc::SYS_time as u32,
            ]);
        }
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            nrs.push(libc::SYS_openat2 as u32);
            nrs.push(libc::SYS_close_range as u32);
            nrs.push(libc::SYS_futex_waitv as u32);
            nrs.push(libc::SYS_epoll_pwait2 as u32);
        }
        nrs.sort_unstable();
        nrs.dedup();
        nrs
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
        for nr in allowed_syscalls() {
            filters.push(jump(jmp_eq, nr, 0, 1));
            filters.push(stmt(ret, libc::SECCOMP_RET_ALLOW));
        }
        filters.push(stmt(ret, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32));
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
    fn seccomp_denies_execve_and_socket() {
        // SAFETY: the child only applies seccomp then exits; the parent waits.
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert_ne!(pid, -1, "fork");
        if pid == 0 {
            let denied = apply().is_ok() && execve_is_denied() && socket_is_denied();
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

    #[cfg(target_os = "linux")]
    fn socket_is_denied() -> bool {
        // SAFETY: a failing socket() only returns an error; no fd is created.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
