//! Shared distribution identity for every Tysel toolchain executable.

mod layout;
mod manifest;

use std::ffi::OsString;

use serde::{Deserialize, Serialize};

pub use layout::{
    INSTALL_STATE_SCHEMA_VERSION, InstallMethod, InstallState, LayoutError, ManagedLayout,
    StateLoadError,
};
pub use manifest::{
    CHANNEL_POINTER_SCHEMA_VERSION, Channel, ChannelPointer, Compatibility, ExpectedFile,
    ManifestError, ManifestLoadError, ManifestSignature, PlatformRequirements,
    RELEASE_MANIFEST_SCHEMA_VERSION, ReleaseAsset, ReleaseManifest, RequiredFeature,
    SignatureAlgorithm,
};

pub const BUILD_INFO_SCHEMA_VERSION: u32 = 1;

/// One canonical developer-toolchain target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    LinuxX64,
    LinuxArm64,
    DarwinX64,
    DarwinArm64,
    Unsupported,
}

impl std::fmt::Display for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.canonical())
    }
}

impl Target {
    pub fn parse(value: &str) -> Option<Self> {
        [Self::LinuxX64, Self::LinuxArm64, Self::DarwinX64, Self::DarwinArm64]
            .into_iter()
            .find(|target| target.accepts(value))
    }

    pub const fn from_canonical(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"linux-x64" => Some(Self::LinuxX64),
            b"linux-arm64" => Some(Self::LinuxArm64),
            b"darwin-x64" => Some(Self::DarwinX64),
            b"darwin-arm64" => Some(Self::DarwinArm64),
            _ => None,
        }
    }

    pub const fn current() -> Self {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return Self::LinuxX64;
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        return Self::LinuxArm64;
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        return Self::DarwinX64;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return Self::DarwinArm64;
        #[cfg(not(any(
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
        )))]
        return Self::Unsupported;
    }

    pub const fn canonical(self) -> &'static str {
        match self {
            Self::LinuxX64 => "linux-x64",
            Self::LinuxArm64 => "linux-arm64",
            Self::DarwinX64 => "darwin-x64",
            Self::DarwinArm64 => "darwin-arm64",
            Self::Unsupported => "unsupported",
        }
    }

    pub const fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::LinuxX64 => &["linux-x64", "linux-amd64", "x86_64-unknown-linux-gnu"],
            Self::LinuxArm64 => &["linux-arm64", "aarch64-unknown-linux-gnu"],
            Self::DarwinX64 => &["darwin-x64", "macos-x64", "x86_64-apple-darwin"],
            Self::DarwinArm64 => &["darwin-arm64", "macos-arm64", "aarch64-apple-darwin"],
            Self::Unsupported => &[],
        }
    }

    pub fn accepts(self, requested: &str) -> bool {
        self.aliases().iter().any(|alias| alias.eq_ignore_ascii_case(requested))
    }
}

/// Stable machine-readable identity emitted by all toolchain executables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildInfo {
    pub schema_version: u32,
    pub binary: String,
    pub version: String,
    pub target: String,
    pub source_commit: Option<String>,
    pub release_id: Option<String>,
}

impl BuildInfo {
    pub fn current(binary: &str, version: &str) -> Self {
        Self {
            schema_version: BUILD_INFO_SCHEMA_VERSION,
            binary: binary.into(),
            version: version.into(),
            target: Target::current().canonical().into(),
            source_commit: option_env!("TYSEL_SOURCE_COMMIT").map(str::to_owned),
            release_id: option_env!("TYSEL_RELEASE_ID").map(str::to_owned),
        }
    }

    pub fn same_release_as(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.version == other.version
            && self.target == other.target
            && self.source_commit == other.source_commit
            && self.release_id == other.release_id
    }
}

/// Render an exact metadata-only invocation without starting the binary's normal work.
pub fn metadata_output(binary: &str, version: &str) -> Option<String> {
    metadata_output_for(binary, version, std::env::args_os().skip(1))
}

pub fn metadata_output_for(
    binary: &str,
    version: &str,
    args: impl IntoIterator<Item = OsString>,
) -> Option<String> {
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [argument] if argument == "--version" => Some(format!("{binary} {version}")),
        [argument] if argument == "--build-info-json" => {
            Some(serde_json::to_string(&BuildInfo::current(binary, version)).expect("build info"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_targets_accept_canonical_names_and_aliases() {
        for target in [Target::LinuxX64, Target::LinuxArm64, Target::DarwinX64, Target::DarwinArm64]
        {
            assert!(target.accepts(target.canonical()));
            assert_eq!(Target::from_canonical(target.canonical()), Some(target));
            for alias in target.aliases() {
                assert!(target.accepts(alias), "{} should accept {alias}", target.canonical());
                assert_eq!(Target::parse(alias), Some(target));
            }
            assert!(!target.accepts("linux-riscv64"));
        }
        assert!(!Target::Unsupported.accepts("unsupported"));
        assert_eq!(Target::from_canonical("linux-amd64"), None);
        assert_eq!(Target::parse("linux-riscv64"), None);
    }

    #[test]
    fn metadata_flags_are_exact_and_machine_readable() {
        assert_eq!(
            metadata_output_for("tysel", "1.2.3", [OsString::from("--version")]).as_deref(),
            Some("tysel 1.2.3")
        );
        let json =
            metadata_output_for("tysel-worker", "1.2.3", [OsString::from("--build-info-json")])
                .unwrap();
        let info: BuildInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info.schema_version, BUILD_INFO_SCHEMA_VERSION);
        assert_eq!(info.binary, "tysel-worker");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(info.target, Target::current().canonical());
        assert!(metadata_output_for("tysel", "1.2.3", []).is_none());
        assert!(
            metadata_output_for(
                "tysel",
                "1.2.3",
                [OsString::from("--version"), OsString::from("extra")]
            )
            .is_none()
        );
    }

    #[test]
    fn release_comparison_ignores_only_binary_name() {
        let mut cli = BuildInfo::current("tysel", "1.2.3");
        let worker = BuildInfo { binary: "tysel-worker".into(), ..cli.clone() };
        assert!(cli.same_release_as(&worker));
        cli.version = "1.2.4".into();
        assert!(!cli.same_release_as(&worker));
    }
}
