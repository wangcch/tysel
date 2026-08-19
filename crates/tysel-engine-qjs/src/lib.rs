//! QuickJS-ng compatibility engine.
//!
//! Spike A pins a runtime to one worker thread and settles promises through a
//! completion queue. Spike B reuses that isolate as a fetch-handler pool behind
//! a native HTTP listener.

use std::path::Path;

mod cpu;
mod fetch;
mod host;
mod isolate;
mod pool;
mod queue;

pub use isolate::{IsolateCancel, eval, eval_cancellable, eval_with_reactor};
pub use pool::{IncomingHttp, IsolatePool};
pub use queue::{IoCompletion, IoRequest, OpId, Reactor, STREAM_WINDOW, open_bridge};

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

#[cfg(test)]
mod tests;
