//! Four-layer capability intersection and deny-by-default policy.
//!
//! Effective permission = Build ∩ App ∩ Deployment ∩ OS (ADR-005). Applications
//! cannot enlarge authority at runtime. Deployment and OS adapters are
//! pass-through until those layers exist.

use tysel_capability::TrustMode;

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Host operations the engine can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cap {
    Sleep,
    Echo,
    SecretRef,
    ReadBody,
    Fetch,
    Sqlite,
    WebSocket,
    Postgres,
    Fs,
    Llm,
}

/// Resolved permission set for one isolate. Construct from an execution
/// profile; do not OR extra capabilities on afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    mode: TrustMode,
}

impl Policy {
    pub fn trusted() -> Self {
        Self { mode: TrustMode::TrustedService }
    }

    pub fn isolated() -> Self {
        Self { mode: TrustMode::IsolatedTask }
    }

    /// `isolated` is the untrusted task mode. Every other profile, including
    /// `service`, is the trusted path.
    pub fn from_profile(profile: &str) -> Self {
        if profile.eq_ignore_ascii_case("isolated") { Self::isolated() } else { Self::trusted() }
    }

    pub fn mode(self) -> TrustMode {
        self.mode
    }

    pub fn allows(self, cap: Cap) -> bool {
        build_allows(cap) && self.app_allows(cap) && deployment_allows(cap) && os_allows(cap)
    }

    pub fn require(self, cap: Cap) -> Result<(), String> {
        if self.allows(cap) { Ok(()) } else { Err(self.deny_message().into()) }
    }

    pub fn deny_message(self) -> &'static str {
        match self.mode {
            TrustMode::IsolatedTask => "capability is not available in the isolated profile",
            TrustMode::TrustedService => "capability is not available",
        }
    }

    fn app_allows(self, cap: Cap) -> bool {
        match self.mode {
            TrustMode::IsolatedTask => {
                matches!(cap, Cap::Sleep | Cap::Echo | Cap::SecretRef | Cap::ReadBody)
            }
            TrustMode::TrustedService => {
                matches!(
                    cap,
                    Cap::Sleep
                        | Cap::Echo
                        | Cap::SecretRef
                        | Cap::ReadBody
                        | Cap::Fetch
                        | Cap::Sqlite
                        | Cap::WebSocket
                        | Cap::Postgres
                        | Cap::Fs
                        | Cap::Llm
                )
            }
        }
    }
}

fn build_allows(_cap: Cap) -> bool {
    true
}

fn deployment_allows(_cap: Cap) -> bool {
    true
}

fn os_allows(_cap: Cap) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use tysel_capability::TrustMode;

    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn isolated_profile_is_deny_by_default() {
        let policy = Policy::from_profile("isolated");
        assert_eq!(policy.mode(), TrustMode::IsolatedTask);
        assert!(policy.allows(Cap::Sleep));
        assert!(policy.allows(Cap::Echo));
        assert!(policy.allows(Cap::SecretRef));
        assert!(policy.allows(Cap::ReadBody));
        assert!(!policy.allows(Cap::Fetch));
        assert!(!policy.allows(Cap::Sqlite));
        assert!(!policy.allows(Cap::WebSocket));
        assert!(!policy.allows(Cap::Postgres));
        assert!(!policy.allows(Cap::Fs));
        assert!(!policy.allows(Cap::Llm));
        assert_eq!(
            policy.require(Cap::Fetch).unwrap_err(),
            "capability is not available in the isolated profile"
        );
    }

    #[test]
    fn trusted_profile_allows_built_caps_only() {
        let policy = Policy::from_profile("service");
        assert_eq!(policy.mode(), TrustMode::TrustedService);
        assert!(policy.allows(Cap::Fetch));
        assert!(policy.allows(Cap::Sqlite));
        assert!(policy.allows(Cap::WebSocket));
        assert!(policy.allows(Cap::Postgres));
        assert!(policy.allows(Cap::Fs));
        assert!(policy.allows(Cap::Llm));
        policy.require(Cap::Fetch).unwrap();
    }

    #[test]
    fn isolated_cannot_regain_fetch_through_intersection() {
        let app = Policy::isolated();
        assert!(build_allows(Cap::Fetch));
        assert!(deployment_allows(Cap::Fetch));
        assert!(os_allows(Cap::Fetch));
        assert!(!app.allows(Cap::Fetch));
    }
}
