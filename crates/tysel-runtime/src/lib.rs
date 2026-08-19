//! Supervisor, reactor, and trusted-service data plane.
//!
//! Spike B owns the native HTTP listener. Spike C runs that listener from a
//! runtime stub that memory-maps an embedded TAP trailer.

mod http;
mod service;

pub use http::{HttpError, bind, bind_with_request_limit, serve};
pub use service::{StubError, run_stub, run_tap};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
