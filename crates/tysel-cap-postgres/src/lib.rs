//! Official Postgres capability.
//!
//! This crate is a scaffolding stub. Behavior lands with the M0 spikes and later
//! milestones in `roadmap.md`.

#![allow(dead_code)]

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }
}
