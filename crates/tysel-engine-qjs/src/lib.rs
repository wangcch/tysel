//! QuickJS-ng compatibility engine.
//!
//! Spike A pins a runtime to one worker thread and settles promises through a
//! completion queue. Spike B reuses that isolate as a fetch-handler pool behind
//! a native HTTP listener.

use std::path::Path;

mod cpu;
mod durable;
mod fetch;
mod fetch_policy;
mod host;
mod isolate;
mod pool;
mod queue;
mod secrets;
mod trust;

pub use durable::DurableSession;
pub use fetch_policy::configure as configure_fetch_hosts;
pub use isolate::{
    IsolateCancel, eval, eval_cancellable, eval_durable, eval_with_reactor,
    eval_with_reactor_deadline,
};
pub use pool::{IncomingHttp, IsolatePool};
pub use queue::{IoCompletion, IoRequest, IoWork, OpId, Reactor, STREAM_WINDOW, open_bridge};
pub use secrets::{
    configure as configure_secrets, load_declared, parse_dotenv, resolve as resolve_secret,
};
pub use trust::configure as configure_policy;

/// Apply the TAP execution profile. `isolated` denies fetch, SQLite,
/// WebSocket, Postgres, and filesystem access; every other profile is the
/// trusted service path.
pub fn configure_execution_profile(profile: &str) {
    trust::configure(tysel_policy::Policy::from_profile(profile));
}

/// Pin the process-wide SQLite file used by trusted-path `tysel.sqlite`.
///
/// An empty path keeps the default in-memory database. Relative paths are
/// resolved against `root` when provided. The first successful call wins.
pub fn configure_sqlite_path(path: &str, root: Option<&Path>) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    if path == ":memory:" || Path::new(path).is_absolute() {
        tysel_cap_sqlite::configure_path(path);
        return;
    }
    let resolved = match root {
        Some(root) => root.join(path).to_string_lossy().into_owned(),
        None => path.to_owned(),
    };
    tysel_cap_sqlite::configure_path(resolved);
}

/// Pin trusted-path filesystem roots. Relative paths are resolved against
/// `root` when provided. Unconfigured processes deny every path.
pub fn configure_fs(read: Vec<String>, write: Vec<String>, root: Option<&Path>) {
    tysel_cap_fs::configure(read, write, root);
}

/// Pin the trusted-path Postgres URL resolved from `TYSEL_POSTGRES_<NAME>`.
/// `None` leaves Postgres unconfigured.
pub fn configure_postgres(url: Option<String>, read_only: bool) {
    tysel_cap_postgres::configure(url, read_only);
}

#[cfg(test)]
mod tests;
