//! Process-wide trust policy for in-process isolates.
//!
//! Unconfigured processes (unit tests) use the trusted service policy. After
//! `configure`, isolated profiles deny fetch, SQLite, WebSocket, Postgres, Redis, and
//! filesystem access even when `[permissions]` lists them. Do not call
//! `configure` from engine-qjs unit tests: they share a process with open-mode
//! tests.

use std::sync::RwLock;

use tysel_policy::{Cap, Policy};

static POLICY: RwLock<Option<Policy>> = RwLock::new(None);

/// Replace the process-wide policy. Tests that never call this keep the
/// trusted mode used by engine unit tests.
pub fn configure(policy: Policy) {
    *POLICY.write().expect("policy lock") = Some(policy);
}

pub fn require(cap: Cap) -> Result<(), String> {
    let guard = POLICY.read().expect("policy lock");
    guard.unwrap_or_else(Policy::trusted).require(cap)
}
