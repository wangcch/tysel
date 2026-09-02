use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use semver::Version;
use tysel_distribution::{
    Channel, ExpectedFile, ManagedLayout, ReleaseAsset, ReleaseManifest, Target,
};

use crate::{integrity::hash_file, upgrade};

const MANIFEST: &str = ".tysel-release-manifest.json";
const MANIFEST_SIGNATURE: &str = ".tysel-release-manifest.json.sig.json";
const TRUST: &str = ".tysel-trust.json";
const TRUST_SIGNATURE: &str = ".tysel-trust.json.sig.json";

pub(crate) fn resolve(target: Target, offline: bool) -> Result<PathBuf> {
    let layout = managed_layout()?;
    resolve_with_layout(&layout, target, offline)
}

fn resolve_with_layout(layout: &ManagedLayout, target: Target, offline: bool) -> Result<PathBuf> {
    let installed_trust = layout.root().join("trust.json");
    upgrade::validate_trust(&installed_trust).with_context(|| {
        "cross-target builds require a managed Tysel trust policy; install Tysel with install.sh \
         or point TYSEL_HOME at a managed installation"
    })?;
    let version = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let cache =
        layout.root().join("build-targets").join(format!("v{version}")).join(target.canonical());
    if let Ok(stub) = verify_cache(&cache, &installed_trust, &version, target) {
        return Ok(stub);
    }
    anyhow::ensure!(!offline, "no verified cached runtime for {target}; rerun without --offline");
    let cache_root = layout.root().join("build-targets");
    fs::create_dir_all(&cache_root).context("create target-runtime cache")?;
    let _lock = upgrade::UpgradeLock::acquire(
        cache_root.join(format!("v{version}-{target}.lock")),
        Duration::from_secs(130),
    )?;
    if let Ok(stub) = verify_cache(&cache, &installed_trust, &version, target) {
        return Ok(stub);
    }

    let staging = layout.staging_dir(&format!("build-target-{}", std::process::id()))?;
    if staging.exists() {
        fs::remove_dir_all(&staging).context("remove stale target-runtime staging directory")?;
    }
    fs::create_dir_all(&staging).context("create target-runtime staging directory")?;
    let result = download(&staging, &installed_trust, &version, target).and_then(|extracted| {
        let parent = cache.parent().context("target-runtime cache has no parent")?;
        fs::create_dir_all(parent)?;
        if cache.exists() {
            fs::remove_dir_all(&cache).context("remove invalid target-runtime cache")?;
        }
        fs::rename(&extracted, &cache).context("publish verified target-runtime cache")?;
        verify_cache(&cache, &installed_trust, &version, target)
    });
    let _ = fs::remove_dir_all(&staging);
    result
}

fn managed_layout() -> Result<ManagedLayout> {
    match upgrade::managed_root() {
        Ok(root) => ManagedLayout::new(root).context("resolve managed Tysel root"),
        Err(_) => ManagedLayout::from_environment()
            .context("resolve managed Tysel root from the running executable or TYSEL_HOME"),
    }
}

fn download(
    staging: &Path,
    installed_trust: &Path,
    version: &Version,
    target: Target,
) -> Result<PathBuf> {
    let client = upgrade::release_client()?;
    eprintln!("Downloading verified Tysel {version} runtime for {target}...");
    let trust = upgrade::resolve_trust_policy(&client, installed_trust, staging)?;
    let (manifest, manifest_path) = upgrade::resolve_manifest(
        &client,
        &trust,
        staging,
        Some(&version.to_string()),
        Channel::Stable,
    )?;
    let minimum = Version::parse(&manifest.minimum_updater_version)
        .context("parse minimum updater version")?;
    anyhow::ensure!(version >= &minimum, "target runtime requires tysel {minimum} or newer");
    let asset = asset(&manifest, target)?;
    let archive = staging.join(format!("tysel-{version}-{target}.tar.gz"));
    upgrade::download_to(&client, &asset.archive_url, &archive, upgrade::MAX_ARCHIVE_BYTES)?;
    anyhow::ensure!(fs::metadata(&archive)?.len() == asset.byte_size, "archive size mismatch");
    anyhow::ensure!(hash_file(&archive)? == asset.sha256, "archive SHA-256 mismatch");
    let archive_signature = archive.with_file_name(format!(
        "{}.sig.json",
        archive.file_name().and_then(|name| name.to_str()).context("archive filename")?
    ));
    upgrade::download_to(&client, &asset.signature.url, &archive_signature, 1024 * 1024)?;
    tysel_build::verify_release_artifact_signature(
        &archive,
        &trust,
        target.canonical(),
        upgrade::now_unix()?,
    )?;
    let extracted = upgrade::extract_archive(&archive, staging, &manifest.version, target)?;
    verify_files(&extracted, asset)?;
    fs::copy(&manifest_path, extracted.join(MANIFEST))?;
    fs::copy(
        staging.join("selected-release-manifest.json.sig.json"),
        extracted.join(MANIFEST_SIGNATURE),
    )?;
    fs::copy(&trust, extracted.join(TRUST))?;
    fs::copy(staging.join("refreshed-trust.json.sig.json"), extracted.join(TRUST_SIGNATURE))?;
    Ok(extracted)
}

fn verify_cache(
    cache: &Path,
    installed_trust: &Path,
    version: &Version,
    target: Target,
) -> Result<PathBuf> {
    let manifest_path = cache.join(MANIFEST);
    let manifest_signature = cache.join(MANIFEST_SIGNATURE);
    let cached_trust = cache.join(TRUST);
    let cached_trust_signature = cache.join(TRUST_SIGNATURE);
    let now = upgrade::now_unix()?;
    if tysel_build::verify_release_metadata_signature(
        &manifest_path,
        &manifest_signature,
        installed_trust,
        now,
    )
    .is_err()
    {
        tysel_build::verify_release_metadata_signature(
            &cached_trust,
            &cached_trust_signature,
            installed_trust,
            now,
        )
        .context("authenticate cached trust policy")?;
        tysel_build::verify_release_metadata_signature(
            &cached_trust,
            &cached_trust_signature,
            &cached_trust,
            now,
        )
        .context("validate cached trust policy")?;
        let installed_policy: tysel_build::TrustPolicy =
            serde_json::from_slice(&fs::read(installed_trust)?)?;
        let cached_policy: tysel_build::TrustPolicy =
            serde_json::from_slice(&fs::read(&cached_trust)?)?;
        tysel_build::validate_trust_policy_transition(&installed_policy, &cached_policy)
            .context("reject unsafe cached trust-policy transition")?;
        tysel_build::verify_release_metadata_signature(
            &manifest_path,
            &manifest_signature,
            &cached_trust,
            now,
        )
        .context("authenticate cached release manifest")?;
    }
    let manifest = ReleaseManifest::from_json(&fs::read(&manifest_path)?)?;
    anyhow::ensure!(manifest.version == version.to_string(), "cached runtime version mismatch");
    let asset = asset(&manifest, target)?;
    verify_files(cache, asset)?;
    Ok(cache.join("bin/tysel-service"))
}

fn asset(manifest: &ReleaseManifest, target: Target) -> Result<&ReleaseAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| asset.target == target)
        .with_context(|| format!("Tysel {} has no runtime for {target}", manifest.version))
}

fn verify_files(root: &Path, asset: &ReleaseAsset) -> Result<()> {
    verify_expected_files(root, &asset.files)
}

fn verify_expected_files(root: &Path, files: &[ExpectedFile]) -> Result<()> {
    for expected in files {
        let path = root.join(&expected.path);
        anyhow::ensure!(path.is_file(), "target runtime is missing {}", expected.path);
        anyhow::ensure!(
            hash_file(&path)? == expected.sha256,
            "target runtime hash mismatch: {}",
            expected.path
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use tysel_build::{
        ReleaseKeyStatus, TRUST_POLICY_VERSION, TrustPolicy, TrustedReleaseKey, release_key_info,
        sign_release_metadata,
    };
    use tysel_distribution::{
        BUILD_INFO_SCHEMA_VERSION, BuildInfo, Compatibility, ManifestSignature,
        PlatformRequirements, RELEASE_MANIFEST_SCHEMA_VERSION, RequiredFeature, SignatureAlgorithm,
    };

    #[test]
    fn target_runtime_files_must_match_the_signed_manifest_hashes() {
        let root =
            std::env::temp_dir().join(format!("tysel-cross-target-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        let service = root.join("bin/tysel-service");
        fs::write(&service, b"verified-stub").unwrap();
        let files = vec![ExpectedFile {
            path: "bin/tysel-service".into(),
            sha256: hash_file(&service).unwrap(),
        }];
        assert!(verify_expected_files(&root, &files).is_ok());
        fs::write(&service, b"tampered-stub").unwrap();
        assert!(
            verify_expected_files(&root, &files).unwrap_err().to_string().contains("hash mismatch")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offline_resolution_reuses_only_a_verified_cached_runtime() {
        let root = temp_root("offline-cache");
        let _ = fs::remove_dir_all(&root);
        let layout = ManagedLayout::new(&root).unwrap();
        let target = foreign_target();
        let version = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let cache = root.join("build-targets").join(format!("v{version}")).join(target.canonical());
        let trust = write_signed_cache(&root, &cache, target, &version);

        let resolved = resolve_with_layout(&layout, target, true).unwrap();
        assert_eq!(resolved, cache.join("bin/tysel-service"));

        fs::write(&resolved, b"tampered-runtime").unwrap();
        let error = resolve_with_layout(&layout, target, true).unwrap_err();
        assert!(error.to_string().contains("no verified cached runtime"), "{error:#}");
        assert!(trust.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offline_resolution_does_not_create_a_missing_cache() {
        let root = temp_root("offline-missing");
        let _ = fs::remove_dir_all(&root);
        let layout = ManagedLayout::new(&root).unwrap();
        let target = foreign_target();
        write_trust(&root);

        let error = resolve_with_layout(&layout, target, true).unwrap_err();
        assert!(error.to_string().contains("no verified cached runtime"), "{error:#}");
        assert!(!root.join("build-targets").exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn write_signed_cache(root: &Path, cache: &Path, target: Target, version: &Version) -> PathBuf {
        let (trust, key, key_id) = write_trust(root);
        fs::create_dir_all(cache.join("bin")).unwrap();
        let binaries = ["tysel", "tysel-service", "tysel-worker"];
        let files = binaries
            .iter()
            .map(|binary| {
                let path = cache.join("bin").join(binary);
                fs::write(&path, format!("verified-{binary}-{target}")).unwrap();
                ExpectedFile { path: format!("bin/{binary}"), sha256: hash_file(&path).unwrap() }
            })
            .collect();
        let source_commit = "01".repeat(20);
        let manifest = ReleaseManifest {
            schema_version: RELEASE_MANIFEST_SCHEMA_VERSION,
            version: version.to_string(),
            source_commit: source_commit.clone(),
            published_at: "2026-09-02T00:00:00Z".into(),
            channel: Channel::Stable,
            minimum_updater_version: "0.1.0".into(),
            compatibility: Compatibility {
                minimum_tap_version: 1,
                maximum_tap_version: 4,
                capability_abi_version: "0.1.0".into(),
                types_version: version.to_string(),
            },
            required_features: vec![RequiredFeature::BuildInfoV1],
            optional_features: BTreeMap::new(),
            assets: vec![ReleaseAsset {
                target,
                archive_url: format!("https://example.invalid/tysel-{version}-{target}.tar.gz"),
                byte_size: 1,
                sha256: "ab".repeat(32),
                signature: ManifestSignature {
                    algorithm: SignatureAlgorithm::Ed25519,
                    url: format!(
                        "https://example.invalid/tysel-{version}-{target}.tar.gz.sig.json"
                    ),
                    key_id,
                },
                files,
                build_info: binaries
                    .iter()
                    .map(|binary| BuildInfo {
                        schema_version: BUILD_INFO_SCHEMA_VERSION,
                        binary: (*binary).into(),
                        version: version.to_string(),
                        target: target.canonical().into(),
                        source_commit: Some(source_commit.clone()),
                        release_id: Some(version.to_string()),
                    })
                    .collect(),
                platform: match target {
                    Target::LinuxX64 | Target::LinuxArm64 => PlatformRequirements {
                        minimum_glibc: Some("2.35".into()),
                        minimum_kernel: Some("5.15".into()),
                        minimum_macos: None,
                    },
                    Target::DarwinX64 | Target::DarwinArm64 => PlatformRequirements {
                        minimum_glibc: None,
                        minimum_kernel: None,
                        minimum_macos: Some("13.0".into()),
                    },
                    Target::Unsupported => unreachable!(),
                },
            }],
        };
        let manifest_path = cache.join(MANIFEST);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let signature = sign_release_metadata(&manifest_path, &key, now()).unwrap();
        fs::rename(signature, cache.join(MANIFEST_SIGNATURE)).unwrap();
        trust
    }

    fn write_trust(root: &Path) -> (PathBuf, PathBuf, String) {
        fs::create_dir_all(root).unwrap();
        let key = root.join("release.key");
        fs::write(&key, format!("{}\n", "11".repeat(32))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let info = release_key_info(&key).unwrap();
        let current = now();
        let trust = root.join("trust.json");
        let policy = TrustPolicy {
            policy_version: TRUST_POLICY_VERSION,
            issued_at_unix: current.saturating_sub(60),
            expires_at_unix: current + 24 * 60 * 60,
            keys: vec![TrustedReleaseKey {
                key_id: info.key_id.clone(),
                algorithm: info.algorithm,
                public_key: info.public_key,
                status: ReleaseKeyStatus::Active,
                valid_from_unix: current.saturating_sub(60),
                valid_until_unix: None,
            }],
        };
        fs::write(&trust, serde_json::to_vec_pretty(&policy).unwrap()).unwrap();
        (trust, key, info.key_id)
    }

    fn foreign_target() -> Target {
        match Target::current() {
            Target::LinuxX64 => Target::LinuxArm64,
            Target::LinuxArm64 => Target::LinuxX64,
            Target::DarwinX64 => Target::DarwinArm64,
            Target::DarwinArm64 | Target::Unsupported => Target::DarwinX64,
        }
    }

    fn now() -> u64 {
        upgrade::now_unix().unwrap()
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tysel-cross-target-{label}-{}-{}",
            std::process::id(),
            now()
        ))
    }
}
