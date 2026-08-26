use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Channel, Target};

pub const INSTALL_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMethod {
    Installer,
    Upgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallState {
    pub schema_version: u32,
    pub active_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    pub channel: Channel,
    pub target: Target,
    pub install_method: InstallMethod,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLayout {
    root: PathBuf,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("TYSEL_HOME or HOME does not resolve to an absolute path")]
    NonAbsoluteRoot,
    #[error("HOME is not set and TYSEL_HOME was not provided")]
    MissingHome,
    #[error("unsupported install state schema version {0}")]
    Schema(u32),
    #[error("invalid semantic version in {0}")]
    Version(&'static str),
    #[error("active and previous versions must differ")]
    SameVersion,
    #[error("unsupported managed installation target")]
    UnsupportedTarget,
    #[error("invalid manifest SHA-256")]
    ManifestSha256,
    #[error("install channel does not match active semantic version")]
    ChannelVersion,
}

impl ManagedLayout {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, LayoutError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(LayoutError::NonAbsoluteRoot);
        }
        Ok(Self { root })
    }

    pub fn from_environment() -> Result<Self, LayoutError> {
        if let Some(root) = std::env::var_os("TYSEL_HOME").filter(|value| !value.is_empty()) {
            return Self::new(root);
        }
        let home = std::env::var_os("HOME").ok_or(LayoutError::MissingHome)?;
        Self::new(PathBuf::from(home).join(".tysel"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, version: &Version) -> PathBuf {
        self.versions_dir().join(format!("v{version}"))
    }

    pub fn version_bin_dir(&self, version: &Version) -> PathBuf {
        self.version_dir(version).join("bin")
    }

    pub fn version_manifest(&self, version: &Version) -> PathBuf {
        self.version_dir(version).join("release-manifest.json")
    }

    pub fn active_bin_link(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn upgrade_lock(&self) -> PathBuf {
        self.root.join("upgrade.lock")
    }

    pub fn staging_dir(&self, transaction_id: &str) -> Result<PathBuf, LayoutError> {
        if transaction_id.is_empty()
            || !transaction_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(LayoutError::Version("transaction ID"));
        }
        Ok(self.root.join(".staging").join(transaction_id))
    }

    pub fn active_bin_target(&self, version: &Version) -> PathBuf {
        PathBuf::from("versions").join(format!("v{version}")).join("bin")
    }
}

impl InstallState {
    pub fn from_json(bytes: &[u8]) -> Result<Self, StateLoadError> {
        let state: Self = serde_json::from_slice(bytes)?;
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.schema_version != INSTALL_STATE_SCHEMA_VERSION {
            return Err(LayoutError::Schema(self.schema_version));
        }
        let active = Version::parse(&self.active_version)
            .map_err(|_| LayoutError::Version("activeVersion"))?;
        let channel_matches = active.build.is_empty()
            && match self.channel {
                Channel::Stable => active.pre.is_empty(),
                Channel::Canary => !active.pre.is_empty(),
            };
        if !channel_matches {
            return Err(LayoutError::ChannelVersion);
        }
        if let Some(previous) = &self.previous_version {
            let previous =
                Version::parse(previous).map_err(|_| LayoutError::Version("previousVersion"))?;
            if active == previous {
                return Err(LayoutError::SameVersion);
            }
        }
        if self.target == Target::Unsupported {
            return Err(LayoutError::UnsupportedTarget);
        }
        if self.manifest_sha256.len() != 64
            || !self
                .manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(LayoutError::ManifestSha256);
        }
        Ok(())
    }

    pub fn active_semver(&self) -> Result<Version, LayoutError> {
        Version::parse(&self.active_version).map_err(|_| LayoutError::Version("activeVersion"))
    }
}

#[derive(Debug, Error)]
pub enum StateLoadError {
    #[error("install state JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] LayoutError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_uses_one_relative_link_for_all_binaries() {
        let layout = ManagedLayout::new("/opt/tysel").unwrap();
        let version = Version::parse("1.2.3-beta.1").unwrap();
        assert_eq!(layout.version_dir(&version), Path::new("/opt/tysel/versions/v1.2.3-beta.1"));
        assert_eq!(layout.active_bin_link(), Path::new("/opt/tysel/bin"));
        assert_eq!(layout.active_bin_target(&version), Path::new("versions/v1.2.3-beta.1/bin"));
        assert_eq!(layout.upgrade_lock(), Path::new("/opt/tysel/upgrade.lock"));
    }

    #[test]
    fn layout_rejects_relative_roots_and_unsafe_staging_ids() {
        assert_eq!(ManagedLayout::new("relative").unwrap_err(), LayoutError::NonAbsoluteRoot);
        let layout = ManagedLayout::new("/opt/tysel").unwrap();
        assert!(layout.staging_dir("../../escape").is_err());
        assert_eq!(
            layout.staging_dir("install-123").unwrap(),
            Path::new("/opt/tysel/.staging/install-123")
        );
    }

    #[test]
    fn state_is_strict_and_keeps_rollback_distinct() {
        let state = InstallState {
            schema_version: 1,
            active_version: "1.2.3".into(),
            previous_version: Some("1.2.2".into()),
            channel: Channel::Stable,
            target: Target::DarwinArm64,
            install_method: InstallMethod::Upgrade,
            manifest_sha256: "ab".repeat(32),
        };
        state.validate().unwrap();
        let encoded = serde_json::to_vec(&state).unwrap();
        assert_eq!(InstallState::from_json(&encoded).unwrap(), state);

        let mut invalid = state.clone();
        invalid.previous_version = Some("1.2.3".into());
        assert_eq!(invalid.validate().unwrap_err(), LayoutError::SameVersion);

        let canary = InstallState {
            active_version: "1.3.0-rc.1".into(),
            previous_version: None,
            channel: Channel::Canary,
            ..state
        };
        canary.validate().unwrap();
        let mismatched = InstallState { channel: Channel::Stable, ..canary };
        assert_eq!(mismatched.validate().unwrap_err(), LayoutError::ChannelVersion);
    }
}
