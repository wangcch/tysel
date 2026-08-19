//! Supervisor, reactor, and trusted-service data plane.
//!
//! Spike B owns the native HTTP listener. Spike C runs that listener from a
//! runtime stub that memory-maps an embedded TAP trailer.

mod http;
mod service;

pub use http::{bind, serve, HttpError};
pub use service::{run_stub, run_tap, StubError};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
