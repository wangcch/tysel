use std::collections::{BTreeMap, HashSet};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BUILD_INFO_SCHEMA_VERSION, BuildInfo, Target};

pub const RELEASE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CHANNEL_POINTER_SCHEMA_VERSION: u32 = 1;
const EXPECTED_BINARIES: [&str; 3] = ["tysel", "tysel-service", "tysel-worker"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Stable,
    Canary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredFeature {
    AtomicBinLink,
    BuildInfoV1,
    Ed25519Manifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestSignature {
    pub algorithm: SignatureAlgorithm,
    pub url: String,
    pub key_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureAlgorithm {
    Ed25519,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_glibc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_kernel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_macos: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Compatibility {
    pub minimum_tap_version: u32,
    pub maximum_tap_version: u32,
    pub capability_abi_version: String,
    pub types_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseAsset {
    pub target: Target,
    pub archive_url: String,
    pub byte_size: u64,
    pub sha256: String,
    pub signature: ManifestSignature,
    pub files: Vec<ExpectedFile>,
    pub build_info: Vec<BuildInfo>,
    pub platform: PlatformRequirements,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub source_commit: String,
    pub published_at: String,
    pub channel: Channel,
    pub minimum_updater_version: String,
    pub compatibility: Compatibility,
    pub required_features: Vec<RequiredFeature>,
    #[serde(default)]
    pub optional_features: BTreeMap<String, serde_json::Value>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelPointer {
    pub schema_version: u32,
    pub channel: Channel,
    pub version: String,
    pub published_at: String,
    pub manifest_url: String,
    pub manifest_byte_size: u64,
    pub manifest_sha256: String,
    pub manifest_signature: ManifestSignature,
    pub required_features: Vec<RequiredFeature>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("unsupported {document} schema version {actual}; expected {expected}")]
    Schema { document: &'static str, actual: u32, expected: u32 },
    #[error("invalid semantic version in {field}: {value}")]
    Version { field: &'static str, value: String },
    #[error("invalid canonical source commit")]
    SourceCommit,
    #[error("invalid publication timestamp")]
    PublishedAt,
    #[error("release manifest must contain at least one asset")]
    NoAssets,
    #[error("duplicate or unsupported release target {0}")]
    Target(Target),
    #[error("invalid HTTPS URL in {0}")]
    Url(&'static str),
    #[error("invalid SHA-256 in {0}")]
    Sha256(&'static str),
    #[error("invalid signature key ID")]
    KeyId,
    #[error("asset byte size must be non-zero")]
    EmptyAsset,
    #[error("asset {target} has invalid expected files: {reason}")]
    Files { target: Target, reason: &'static str },
    #[error("asset {target} has invalid build identities: {reason}")]
    BuildInfo { target: Target, reason: &'static str },
    #[error("invalid compatibility contract: {0}")]
    Compatibility(&'static str),
    #[error("invalid platform requirements for {0}")]
    Platform(Target),
    #[error("duplicate required feature")]
    DuplicateFeature,
    #[error("{document} channel {channel:?} does not match semantic version {version}")]
    ChannelVersion { document: &'static str, channel: Channel, version: String },
}

impl ReleaseManifest {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestLoadError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        check_schema("release manifest", self.schema_version, RELEASE_MANIFEST_SCHEMA_VERSION)?;
        let version = parse_version("version", &self.version)?;
        check_channel_version("release manifest", self.channel, &version)?;
        parse_version("minimumUpdaterVersion", &self.minimum_updater_version)?;
        check_commit(&self.source_commit)?;
        check_timestamp(&self.published_at)?;
        check_features(&self.required_features)?;
        self.compatibility.validate()?;
        if self.assets.is_empty() {
            return Err(ManifestError::NoAssets);
        }
        let mut targets = HashSet::new();
        for asset in &self.assets {
            if asset.target == Target::Unsupported || !targets.insert(asset.target) {
                return Err(ManifestError::Target(asset.target));
            }
            asset.validate(self)?;
        }
        Ok(())
    }
}

impl ChannelPointer {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestLoadError> {
        let pointer: Self = serde_json::from_slice(bytes)?;
        pointer.validate()?;
        Ok(pointer)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        check_schema("channel pointer", self.schema_version, CHANNEL_POINTER_SCHEMA_VERSION)?;
        let version = parse_version("version", &self.version)?;
        check_channel_version("channel pointer", self.channel, &version)?;
        check_timestamp(&self.published_at)?;
        check_https(&self.manifest_url, "manifestUrl")?;
        if self.manifest_byte_size == 0 {
            return Err(ManifestError::EmptyAsset);
        }
        check_sha256(&self.manifest_sha256, "manifestSha256")?;
        self.manifest_signature.validate()?;
        check_features(&self.required_features)
    }
}

fn check_channel_version(
    document: &'static str,
    channel: Channel,
    version: &Version,
) -> Result<(), ManifestError> {
    let matches = version.build.is_empty()
        && match channel {
            Channel::Stable => version.pre.is_empty(),
            Channel::Canary => !version.pre.is_empty(),
        };
    if matches {
        Ok(())
    } else {
        Err(ManifestError::ChannelVersion { document, channel, version: version.to_string() })
    }
}

impl Compatibility {
    fn validate(&self) -> Result<(), ManifestError> {
        if self.minimum_tap_version == 0 || self.minimum_tap_version > self.maximum_tap_version {
            return Err(ManifestError::Compatibility("invalid TAP version range"));
        }
        parse_version("capabilityAbiVersion", &self.capability_abi_version)?;
        parse_version("typesVersion", &self.types_version)?;
        Ok(())
    }
}

impl ReleaseAsset {
    fn validate(&self, manifest: &ReleaseManifest) -> Result<(), ManifestError> {
        check_https(&self.archive_url, "archiveUrl")?;
        if self.byte_size == 0 {
            return Err(ManifestError::EmptyAsset);
        }
        check_sha256(&self.sha256, "asset.sha256")?;
        self.signature.validate()?;
        self.validate_files()?;
        self.validate_build_info(manifest)?;
        self.platform.validate(self.target)
    }

    fn validate_files(&self) -> Result<(), ManifestError> {
        let mut found = HashSet::new();
        for file in &self.files {
            if !EXPECTED_BINARIES.iter().any(|binary| file.path == format!("bin/{binary}")) {
                return Err(ManifestError::Files {
                    target: self.target,
                    reason: "only the three managed binary paths are allowed",
                });
            }
            if !found.insert(file.path.as_str()) {
                return Err(ManifestError::Files {
                    target: self.target,
                    reason: "duplicate binary path",
                });
            }
            check_sha256(&file.sha256, "asset.files.sha256")?;
        }
        if found.len() != EXPECTED_BINARIES.len() {
            return Err(ManifestError::Files {
                target: self.target,
                reason: "all three managed binaries are required",
            });
        }
        Ok(())
    }

    fn validate_build_info(&self, manifest: &ReleaseManifest) -> Result<(), ManifestError> {
        if self.build_info.len() != EXPECTED_BINARIES.len() {
            return Err(ManifestError::BuildInfo {
                target: self.target,
                reason: "all three build identities are required",
            });
        }
        let mut binaries = HashSet::new();
        for info in &self.build_info {
            if info.schema_version != BUILD_INFO_SCHEMA_VERSION
                || !EXPECTED_BINARIES.contains(&info.binary.as_str())
                || !binaries.insert(info.binary.as_str())
                || info.version != manifest.version
                || info.target != self.target.canonical()
                || info.source_commit.as_deref() != Some(&manifest.source_commit)
                || info.release_id.as_deref() != Some(&manifest.version)
            {
                return Err(ManifestError::BuildInfo {
                    target: self.target,
                    reason: "identity does not match the release",
                });
            }
        }
        let first = &self.build_info[0];
        if self.build_info[1..].iter().any(|info| !first.same_release_as(info)) {
            return Err(ManifestError::BuildInfo {
                target: self.target,
                reason: "mixed release identities",
            });
        }
        Ok(())
    }
}

impl PlatformRequirements {
    fn validate(&self, target: Target) -> Result<(), ManifestError> {
        let valid = match target {
            Target::LinuxX64 | Target::LinuxArm64 => {
                self.minimum_glibc.as_deref().is_some_and(valid_platform_version)
                    && self.minimum_kernel.as_deref().is_some_and(valid_platform_version)
                    && self.minimum_macos.is_none()
            }
            Target::DarwinX64 | Target::DarwinArm64 => {
                self.minimum_macos.as_deref().is_some_and(valid_platform_version)
                    && self.minimum_glibc.is_none()
                    && self.minimum_kernel.is_none()
            }
            Target::Unsupported => false,
        };
        if !valid {
            return Err(ManifestError::Platform(target));
        }
        Ok(())
    }
}

impl ManifestSignature {
    fn validate(&self) -> Result<(), ManifestError> {
        check_https(&self.url, "signature.url")?;
        if !is_lower_hex(&self.key_id, 64) {
            return Err(ManifestError::KeyId);
        }
        Ok(())
    }
}

fn check_schema(document: &'static str, actual: u32, expected: u32) -> Result<(), ManifestError> {
    if actual != expected {
        return Err(ManifestError::Schema { document, actual, expected });
    }
    Ok(())
}

fn parse_version(field: &'static str, value: &str) -> Result<Version, ManifestError> {
    Version::parse(value).map_err(|_| ManifestError::Version { field, value: value.into() })
}

fn check_commit(value: &str) -> Result<(), ManifestError> {
    if !is_lower_hex(value, 40) {
        return Err(ManifestError::SourceCommit);
    }
    Ok(())
}

fn check_sha256(value: &str, field: &'static str) -> Result<(), ManifestError> {
    if !is_lower_hex(value, 64) {
        return Err(ManifestError::Sha256(field));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn check_https(value: &str, field: &'static str) -> Result<(), ManifestError> {
    if !value.starts_with("https://")
        || value.len() <= "https://".len()
        || value.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(ManifestError::Url(field));
    }
    Ok(())
}

fn check_timestamp(value: &str) -> Result<(), ManifestError> {
    if value.len() < 20 || !value.contains('T') || !value.ends_with('Z') {
        return Err(ManifestError::PublishedAt);
    }
    Ok(())
}

fn valid_platform_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    (2..=4).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn check_features(features: &[RequiredFeature]) -> Result<(), ManifestError> {
    let mut found = HashSet::new();
    if features.iter().any(|feature| !found.insert(*feature)) {
        return Err(ManifestError::DuplicateFeature);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ManifestLoadError {
    #[error("release metadata JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] ManifestError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha() -> String {
        "ab".repeat(32)
    }

    fn build_info(binary: &str) -> BuildInfo {
        BuildInfo {
            schema_version: 1,
            binary: binary.into(),
            version: "1.2.3".into(),
            target: "linux-x64".into(),
            source_commit: Some("01".repeat(20)),
            release_id: Some("1.2.3".into()),
        }
    }

    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema_version: 1,
            version: "1.2.3".into(),
            source_commit: "01".repeat(20),
            published_at: "2026-08-22T12:00:00Z".into(),
            channel: Channel::Stable,
            minimum_updater_version: "0.0.1".into(),
            compatibility: Compatibility {
                minimum_tap_version: 1,
                maximum_tap_version: 3,
                capability_abi_version: "0.4.0".into(),
                types_version: "1.2.3".into(),
            },
            required_features: vec![RequiredFeature::BuildInfoV1],
            optional_features: BTreeMap::new(),
            assets: vec![ReleaseAsset {
                target: Target::LinuxX64,
                archive_url: "https://tysel.dev/releases/v1.2.3/tysel-linux-x64.tar.gz".into(),
                byte_size: 42,
                sha256: sha(),
                signature: ManifestSignature {
                    algorithm: SignatureAlgorithm::Ed25519,
                    url: "https://tysel.dev/releases/v1.2.3/tysel-linux-x64.tar.gz.sig.json".into(),
                    key_id: sha(),
                },
                files: EXPECTED_BINARIES
                    .iter()
                    .map(|binary| ExpectedFile { path: format!("bin/{binary}"), sha256: sha() })
                    .collect(),
                build_info: EXPECTED_BINARIES.iter().map(|binary| build_info(binary)).collect(),
                platform: PlatformRequirements {
                    minimum_glibc: Some("2.35".into()),
                    minimum_kernel: Some("5.15".into()),
                    minimum_macos: None,
                },
            }],
        }
    }

    #[test]
    fn valid_release_round_trips_through_strict_json() {
        let expected = manifest();
        let json = serde_json::to_vec(&expected).unwrap();
        assert_eq!(ReleaseManifest::from_json(&json).unwrap(), expected);
    }

    #[test]
    fn semantic_version_intrinsically_selects_the_release_channel() {
        let mut release = manifest();
        release.version = "1.2.3-canary.4".into();
        release.compatibility.types_version = release.version.clone();
        for build in &mut release.assets[0].build_info {
            build.version = release.version.clone();
            build.release_id = Some(release.version.clone());
        }
        assert!(matches!(release.validate(), Err(ManifestError::ChannelVersion { .. })));
        release.channel = Channel::Canary;
        release.validate().unwrap();

        release.version = "1.2.3".into();
        assert!(matches!(release.validate(), Err(ManifestError::ChannelVersion { .. })));
    }

    #[test]
    fn unknown_fields_and_targets_fail_closed() {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value.as_object_mut().unwrap().insert("futureTrust".into(), true.into());
        assert!(ReleaseManifest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

        let mut value = serde_json::to_value(manifest()).unwrap();
        value["assets"][0]["target"] = "linux-riscv64".into();
        assert!(ReleaseManifest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn mixed_or_incomplete_binary_sets_are_rejected() {
        let mut release = manifest();
        release.assets[0].build_info[2].version = "1.2.4".into();
        assert!(matches!(release.validate(), Err(ManifestError::BuildInfo { .. })));

        let mut release = manifest();
        release.assets[0].files.pop();
        assert!(matches!(release.validate(), Err(ManifestError::Files { .. })));
    }

    #[test]
    fn platform_requirements_are_mandatory_and_target_specific() {
        let mut release = manifest();
        release.assets[0].platform.minimum_glibc = None;
        assert_eq!(release.validate(), Err(ManifestError::Platform(Target::LinuxX64)));

        let mut release = manifest();
        release.assets[0].platform.minimum_macos = Some("13.0".into());
        assert_eq!(release.validate(), Err(ManifestError::Platform(Target::LinuxX64)));

        let mut release = manifest();
        release.assets[0].platform.minimum_kernel = Some("rolling".into());
        assert_eq!(release.validate(), Err(ManifestError::Platform(Target::LinuxX64)));
    }

    #[test]
    fn channel_pointer_is_strict_and_authenticated_by_reference() {
        let pointer = ChannelPointer {
            schema_version: 1,
            channel: Channel::Stable,
            version: "1.2.3".into(),
            published_at: "2026-08-22T12:00:00Z".into(),
            manifest_url: "https://tysel.dev/releases/v1.2.3/release-manifest.json".into(),
            manifest_byte_size: 100,
            manifest_sha256: sha(),
            manifest_signature: ManifestSignature {
                algorithm: SignatureAlgorithm::Ed25519,
                url: "https://tysel.dev/releases/v1.2.3/release-manifest.json.sig.json".into(),
                key_id: sha(),
            },
            required_features: vec![RequiredFeature::Ed25519Manifest],
        };
        pointer.validate().unwrap();
        let mut value = serde_json::to_value(pointer).unwrap();
        value["manifestSignature"]["algorithm"] = "rsa".into();
        assert!(ChannelPointer::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
