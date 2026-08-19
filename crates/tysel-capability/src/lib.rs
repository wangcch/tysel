//! Capability identifiers and the four-layer permission model.
//!
//! Effective permission = Build ∩ App Request ∩ Deployment Policy ∩ OS Boundary.
//! Applications cannot enlarge authority at runtime.

#![allow(dead_code)]

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CapabilityId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustMode {
    TrustedService,
    IsolatedTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub id: CapabilityId,
    pub resources: Vec<String>,
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_id_roundtrip() {
        let id = CapabilityId("tysel:http".into());
        assert_eq!(id.0, "tysel:http");
    }
}
