use crate::supervisor::IsolateError;

pub fn apply_resource_limits(as_bytes: usize) -> Result<(), IsolateError> {
    #[cfg(target_os = "linux")]
    {
        let limit = as_bytes.max(1) as libc::rlim_t;
        let as_limit = libc::rlimit { rlim_cur: limit, rlim_max: limit };
        let nofile = libc::rlimit { rlim_cur: 64, rlim_max: 64 };
        // SAFETY: setrlimit only mutates this process's resource limits.
        #[allow(unsafe_code)]
        let as_rc = unsafe { libc::setrlimit(libc::RLIMIT_AS, &as_limit) };
        if as_rc != 0 {
            return Err(IsolateError::Limit(std::io::Error::last_os_error().to_string()));
        }
        #[allow(unsafe_code)]
        let nofile_rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &nofile) };
        if nofile_rc != 0 {
            return Err(IsolateError::Limit(std::io::Error::last_os_error().to_string()));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = as_bytes;
    Ok(())
}
