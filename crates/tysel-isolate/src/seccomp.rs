//! Linux seccomp filter for isolated workers.
//!
//! Isolated workers are restricted to an allowlist: unmatched syscalls return
//! EPERM, and a mismatched architecture kills the process. The list is enough
//! for QuickJS, Rust threads, and the shared Tokio I/O runtime. Local
//! `socketpair` IPC is allowed for Tokio's signal driver, while network sockets,
//! exec, ptrace, mount, and bpf remain denied. macOS is not the security gate of
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
    use std::mem::{offset_of, size_of};

    use super::IsolateError;

    const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;
    const SOCK_TYPE_MASK: u32 = 0x0f;
    const SOCK_ALLOWED_FLAGS: u32 = libc::SOCK_CLOEXEC as u32 | libc::SOCK_NONBLOCK as u32;

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
        let alu_and = (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16;
        let ret = (libc::BPF_RET | libc::BPF_K) as u16;
        let mut filters = vec![
            stmt(ld_abs, offset_of!(libc::seccomp_data, arch) as u32),
            jump(jmp_eq, AUDIT_ARCH, 1, 0),
            stmt(ret, libc::SECCOMP_RET_KILL_PROCESS),
            stmt(ld_abs, offset_of!(libc::seccomp_data, nr) as u32),
            // socketpair is needed by Tokio's signal driver, but only as a
            // local stream pair. Skip this twelve-instruction rule for every
            // other syscall, then reload nr for the ordinary allowlist.
            jump(jmp_eq, libc::SYS_socketpair as u32, 0, 12),
            stmt(ld_abs, arg_low_offset(0)),
            jump(jmp_eq, libc::AF_UNIX as u32, 0, 9),
            stmt(ld_abs, arg_low_offset(1)),
            stmt(alu_and, SOCK_TYPE_MASK),
            jump(jmp_eq, libc::SOCK_STREAM as u32, 0, 6),
            stmt(ld_abs, arg_low_offset(1)),
            stmt(alu_and, !(SOCK_TYPE_MASK | SOCK_ALLOWED_FLAGS)),
            jump(jmp_eq, 0, 0, 3),
            stmt(ld_abs, arg_low_offset(2)),
            jump(jmp_eq, 0, 0, 1),
            stmt(ret, libc::SECCOMP_RET_ALLOW),
            stmt(ret, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
            stmt(ld_abs, offset_of!(libc::seccomp_data, nr) as u32),
        ];
        for nr in allowed_syscalls() {
            filters.push(jump(jmp_eq, nr, 0, 1));
            filters.push(stmt(ret, libc::SECCOMP_RET_ALLOW));
        }
        filters.push(stmt(ret, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32));
        filters
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn arg_low_offset(index: usize) -> u32 {
        let word_offset = if cfg!(target_endian = "little") { 0 } else { 4 };
        (offset_of!(libc::seccomp_data, args) + index * size_of::<u64>() + word_offset) as u32
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
    fn seccomp_denies_execve_and_network_socket_but_allows_unix_pair() {
        // SAFETY: the child only applies seccomp then exits; the parent waits.
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert_ne!(pid, -1, "fork");
        if pid == 0 {
            let denied = apply().is_ok()
                && execve_is_denied()
                && network_socket_is_denied()
                && unix_socketpair_is_allowed()
                && tipc_socketpair_is_denied()
                && unix_datagram_socketpair_is_denied()
                && unix_nonzero_protocol_socketpair_is_denied();
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
    fn network_socket_is_denied() -> bool {
        // SAFETY: a failing socket() only returns an error; no fd is created.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(target_os = "linux")]
    fn unix_socketpair_is_allowed() -> bool {
        let mut fds = [-1, -1];
        // SAFETY: fds has room for the two descriptors returned by socketpair.
        #[allow(unsafe_code)]
        let rc = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
                fds.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return false;
        }
        // SAFETY: successful socketpair initialized both descriptors.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        true
    }

    #[cfg(target_os = "linux")]
    fn tipc_socketpair_is_denied() -> bool {
        socketpair_is_denied(libc::AF_TIPC, libc::SOCK_STREAM, 0)
    }

    #[cfg(target_os = "linux")]
    fn unix_datagram_socketpair_is_denied() -> bool {
        socketpair_is_denied(libc::AF_UNIX, libc::SOCK_DGRAM, 0)
    }

    #[cfg(target_os = "linux")]
    fn unix_nonzero_protocol_socketpair_is_denied() -> bool {
        socketpair_is_denied(libc::AF_UNIX, libc::SOCK_STREAM, 1)
    }

    #[cfg(target_os = "linux")]
    fn socketpair_is_denied(domain: libc::c_int, kind: libc::c_int, protocol: libc::c_int) -> bool {
        let mut fds = [-1, -1];
        // SAFETY: fds has room for two descriptors; denied calls create none.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::socketpair(domain, kind, protocol, fds.as_mut_ptr()) };
        rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
