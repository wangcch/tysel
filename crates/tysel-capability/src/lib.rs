//! Capability identifiers and the four-layer permission model.
//!
//! Effective permission = Build ∩ App Request ∩ Deployment Policy ∩ OS Boundary.
//! Applications cannot enlarge authority at runtime.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

const MAX_CAPABILITY_ID_BYTES: usize = 128;
const MAX_INTERFACE_BYTES: usize = 128;
const MAX_CAPABILITY_IMPORT_BYTES: usize = 384;
const MAX_ABI_VERSION_BYTES: usize = 64;

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct CapabilityId(pub String);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum TrustMode {
    TrustedService,
    IsolatedTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub id: CapabilityId,
    pub resources: Vec<String>,
}

/// Stable, prerelease-free WIT ABI version. v0 compatibility is intentionally
/// strict: the minor version is the compatibility boundary until v1.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct AbiVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl AbiVersion {
    pub fn is_compatible_provider_for(self, requested: Self) -> bool {
        if self < requested || self.major != requested.major {
            return false;
        }
        requested.major != 0 || self.minor == requested.minor
    }
}

impl FromStr for AbiVersion {
    type Err = CapabilityRegistryError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        if source.len() > MAX_ABI_VERSION_BYTES {
            return Err(CapabilityRegistryError::InvalidVersion(source.into()));
        }
        let version = semver::Version::parse(source)
            .map_err(|_| CapabilityRegistryError::InvalidVersion(source.into()))?;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(CapabilityRegistryError::InvalidVersion(source.into()));
        }
        Ok(Self { major: version.major, minor: version.minor, patch: version.patch })
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One version-qualified WIT interface import such as
/// `tysel:http/outgoing@0.4.0`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityImport {
    pub id: CapabilityId,
    pub interface: String,
    pub version: AbiVersion,
}

impl FromStr for CapabilityImport {
    type Err = CapabilityRegistryError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        if source.len() > MAX_CAPABILITY_IMPORT_BYTES {
            return Err(CapabilityRegistryError::InvalidImport(source.into()));
        }
        let (name, version) = source
            .rsplit_once('@')
            .ok_or_else(|| CapabilityRegistryError::InvalidImport(source.into()))?;
        let (id, interface) = name
            .split_once('/')
            .ok_or_else(|| CapabilityRegistryError::InvalidImport(source.into()))?;
        validate_capability_id(id)?;
        validate_identifier("interface", interface, MAX_INTERFACE_BYTES)?;
        Ok(Self {
            id: CapabilityId(id.into()),
            interface: interface.into(),
            version: version.parse()?,
        })
    }
}

impl fmt::Display for CapabilityImport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}@{}", self.id.0, self.interface, self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub import: CapabilityImport,
    pub trust_modes: BTreeSet<TrustMode>,
}

impl CapabilityDescriptor {
    pub fn new(
        import: CapabilityImport,
        trust_modes: impl IntoIterator<Item = TrustMode>,
    ) -> Result<Self, CapabilityRegistryError> {
        let trust_modes = trust_modes.into_iter().collect::<BTreeSet<_>>();
        if trust_modes.is_empty() {
            return Err(CapabilityRegistryError::NoTrustModes(import.to_string()));
        }
        Ok(Self { import, trust_modes })
    }
}

/// Deterministic metadata registry used before any implementation is added to
/// a Wasmtime linker. Registration and resolution fail closed.
#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    entries: BTreeMap<(CapabilityId, String), Vec<CapabilityDescriptor>>,
}

impl CapabilityRegistry {
    pub fn new(
        descriptors: impl IntoIterator<Item = CapabilityDescriptor>,
    ) -> Result<Self, CapabilityRegistryError> {
        let mut entries: BTreeMap<_, Vec<CapabilityDescriptor>> = BTreeMap::new();
        for descriptor in descriptors {
            let key = (descriptor.import.id.clone(), descriptor.import.interface.clone());
            let versions = entries.entry(key).or_default();
            if versions.iter().any(|existing| existing.import.version == descriptor.import.version)
            {
                return Err(CapabilityRegistryError::Duplicate(descriptor.import.to_string()));
            }
            versions.push(descriptor);
            versions.sort_by_key(|descriptor| descriptor.import.version);
        }
        Ok(Self { entries })
    }

    pub fn resolve(
        &self,
        requested: &CapabilityImport,
        trust_mode: TrustMode,
        effective_grants: &BTreeSet<CapabilityId>,
    ) -> Result<&CapabilityDescriptor, CapabilityRegistryError> {
        if !effective_grants.contains(&requested.id) {
            return Err(CapabilityRegistryError::Denied(requested.to_string()));
        }
        let versions = self
            .entries
            .get(&(requested.id.clone(), requested.interface.clone()))
            .ok_or_else(|| CapabilityRegistryError::Unknown(requested.to_string()))?;
        if !versions.iter().any(|descriptor| {
            descriptor.import.version.is_compatible_provider_for(requested.version)
        }) {
            return Err(CapabilityRegistryError::Incompatible(requested.to_string()));
        }
        versions
            .iter()
            .rev()
            .filter(|descriptor| {
                descriptor.import.version.is_compatible_provider_for(requested.version)
            })
            .find(|descriptor| descriptor.trust_modes.contains(&trust_mode))
            .ok_or_else(|| CapabilityRegistryError::Denied(requested.to_string()))
    }

    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute the runtime authority visible to a Component. Omitting a layer is
/// equivalent to granting nothing, never to bypassing that layer.
pub fn effective_grants(
    build: &BTreeSet<CapabilityId>,
    application: &BTreeSet<CapabilityId>,
    deployment: &BTreeSet<CapabilityId>,
) -> BTreeSet<CapabilityId> {
    build.intersection(application).filter(|id| deployment.contains(*id)).cloned().collect()
}

fn validate_capability_id(value: &str) -> Result<(), CapabilityRegistryError> {
    let Some((namespace, name)) = value.split_once(':') else {
        return Err(CapabilityRegistryError::InvalidCapabilityId(value.into()));
    };
    if value.len() > MAX_CAPABILITY_ID_BYTES
        || !valid_identifier(namespace)
        || !valid_identifier(name)
    {
        return Err(CapabilityRegistryError::InvalidCapabilityId(value.into()));
    }
    Ok(())
}

fn validate_identifier(
    label: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), CapabilityRegistryError> {
    if value.len() > maximum || !valid_identifier(value) {
        return Err(CapabilityRegistryError::InvalidIdentifier { label, value: value.into() });
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityRegistryError {
    #[error("invalid capability import '{0}'")]
    InvalidImport(String),
    #[error("invalid capability id '{0}'")]
    InvalidCapabilityId(String),
    #[error("invalid {label} identifier '{value}'")]
    InvalidIdentifier { label: &'static str, value: String },
    #[error("invalid capability ABI version '{0}'")]
    InvalidVersion(String),
    #[error("capability '{0}' has no allowed trust modes")]
    NoTrustModes(String),
    #[error("duplicate capability registration '{0}'")]
    Duplicate(String),
    #[error("unknown capability '{0}'")]
    Unknown(String),
    #[error("no compatible capability ABI for '{0}'")]
    Incompatible(String),
    #[error("capability '{0}' is not granted")]
    Denied(String),
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

    fn descriptor(source: &str, modes: &[TrustMode]) -> CapabilityDescriptor {
        CapabilityDescriptor::new(source.parse().unwrap(), modes.iter().copied()).unwrap()
    }

    fn grants(ids: &[&str]) -> BTreeSet<CapabilityId> {
        ids.iter().map(|id| CapabilityId((*id).into())).collect()
    }

    #[test]
    fn parses_and_formats_versioned_wit_imports() {
        let import: CapabilityImport = "tysel:http/outgoing@0.4.2".parse().unwrap();
        assert_eq!(import.id, CapabilityId("tysel:http".into()));
        assert_eq!(import.interface, "outgoing");
        assert_eq!(import.version, AbiVersion { major: 0, minor: 4, patch: 2 });
        assert_eq!(import.to_string(), "tysel:http/outgoing@0.4.2");
        for invalid in ["http/outgoing@0.4.0", "tysel:http/outgoing", "tysel:http/OUT@0.4.0"] {
            assert!(invalid.parse::<CapabilityImport>().is_err(), "accepted {invalid}");
        }
        assert!("x".repeat(MAX_CAPABILITY_IMPORT_BYTES + 1).parse::<CapabilityImport>().is_err());
    }

    #[test]
    fn registry_selects_latest_compatible_version_deterministically() {
        let registry = CapabilityRegistry::new([
            descriptor("tysel:http/outgoing@0.4.1", &[TrustMode::TrustedService]),
            descriptor("tysel:http/outgoing@0.4.3", &[TrustMode::TrustedService]),
            descriptor("tysel:http/outgoing@0.5.0", &[TrustMode::TrustedService]),
        ])
        .unwrap();
        let requested = "tysel:http/outgoing@0.4.0".parse().unwrap();
        let resolved = registry
            .resolve(&requested, TrustMode::TrustedService, &grants(&["tysel:http"]))
            .unwrap();
        assert_eq!(resolved.import.version, AbiVersion { major: 0, minor: 4, patch: 3 });
    }

    #[test]
    fn registry_selects_latest_compatible_version_allowed_for_the_trust_mode() {
        let registry = CapabilityRegistry::new([
            descriptor("tysel:http/outgoing@0.4.1", &[TrustMode::IsolatedTask]),
            descriptor("tysel:http/outgoing@0.4.3", &[TrustMode::TrustedService]),
        ])
        .unwrap();
        let requested = "tysel:http/outgoing@0.4.0".parse().unwrap();
        let resolved = registry
            .resolve(&requested, TrustMode::IsolatedTask, &grants(&["tysel:http"]))
            .unwrap();
        assert_eq!(resolved.import.version, AbiVersion { major: 0, minor: 4, patch: 1 });
    }

    #[test]
    fn registry_rejects_duplicates_incompatible_versions_and_ungranted_modes() {
        let duplicate = descriptor("tysel:http/outgoing@0.4.0", &[TrustMode::TrustedService]);
        assert!(matches!(
            CapabilityRegistry::new([duplicate.clone(), duplicate]),
            Err(CapabilityRegistryError::Duplicate(_))
        ));

        let registry = CapabilityRegistry::new([descriptor(
            "tysel:http/outgoing@0.5.0",
            &[TrustMode::TrustedService],
        )])
        .unwrap();
        let requested = "tysel:http/outgoing@0.4.0".parse().unwrap();
        assert!(matches!(
            registry.resolve(&requested, TrustMode::TrustedService, &grants(&["tysel:http"])),
            Err(CapabilityRegistryError::Incompatible(_))
        ));
        assert!(matches!(
            registry.resolve(&requested, TrustMode::TrustedService, &BTreeSet::new()),
            Err(CapabilityRegistryError::Denied(_))
        ));
    }

    #[test]
    fn effective_authority_is_the_intersection_of_all_layers() {
        let effective = effective_grants(
            &grants(&["tysel:http", "tysel:llm"]),
            &grants(&["tysel:http", "tysel:database"]),
            &grants(&["tysel:http", "tysel:llm"]),
        );
        assert_eq!(effective, grants(&["tysel:http"]));
    }
}
