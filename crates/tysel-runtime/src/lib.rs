//! Supervisor, reactor, and trusted-service data plane.
//!
//! Spike B owns the native HTTP listener and dispatches each request to a
//! QuickJS fetch handler through `tysel-engine-qjs`.

mod http;

pub use http::{bind, serve, HttpError};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
