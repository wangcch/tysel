//! Process-isolated worker, supervisor, and POSIX resource limits.
//!
//! Spike D runs untrusted QuickJS in a child process. Host I/O and secrets stay
//! in the supervisor capability broker and cross a bounded IPC pipe.

mod broker;
mod rlimit;
mod supervisor;
mod worker;

pub use supervisor::{IsolateError, Supervisor, WorkerSpec};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

pub fn worker_main() -> Result<(), IsolateError> {
    worker::run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }
}
