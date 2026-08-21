use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tysel_package::bundle_hash;

use crate::evidence::write_json_atomically;
use crate::supply_chain::{embedded_runtime_inventory, inventory_digest};

pub const REPRODUCIBLE_BUILD_EVIDENCE_VERSION: u32 = 1;
const MAX_REPRODUCIBLE_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PROVENANCE_VALUE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReproducibleBuildEvidence {
    pub evidence_version: u32,
    pub source_commit: String,
    pub target: String,
    pub toolchain: String,
    pub cargo_lock_sha256: String,
    pub runtime_inventory_sha256: String,
    pub artifact: ReproducibleArtifact,
    pub builds: Vec<ReproducibleBuild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReproducibleArtifact {
    pub kind: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReproducibleBuild {
    pub ordinal: u8,
    pub command: String,
    pub artifact_sha256: String,
}

pub fn compare_reproducible_builds(
    first: impl AsRef<Path>,
    second: impl AsRef<Path>,
    source_commit: &str,
    target: &str,
    toolchain: &str,
    commands: &[String],
    cargo_lock: impl AsRef<Path>,
) -> Result<ReproducibleBuildEvidence> {
    validate_provenance(source_commit, target, toolchain, commands)?;
    let first = read_artifact(first.as_ref())?;
    let second = read_artifact(second.as_ref())?;
    let first_sha256 = bundle_hash(&first);
    let second_sha256 = bundle_hash(&second);
    ensure!(
        first == second,
        "release builds are not reproducible: first {first_sha256}, second {second_sha256}"
    );

    let cargo_lock = fs::read(cargo_lock.as_ref())
        .with_context(|| format!("failed to read {}", cargo_lock.as_ref().display()))?;
    let cargo_lock_sha256 = bundle_hash(&cargo_lock);
    let inventory = embedded_runtime_inventory()?;
    ensure!(
        cargo_lock_sha256 == inventory.cargo_lock_sha256,
        "Cargo.lock does not match the embedded runtime inventory"
    );

    Ok(ReproducibleBuildEvidence {
        evidence_version: REPRODUCIBLE_BUILD_EVIDENCE_VERSION,
        source_commit: source_commit.into(),
        target: target.into(),
        toolchain: toolchain.into(),
        cargo_lock_sha256,
        runtime_inventory_sha256: inventory_digest(),
        artifact: ReproducibleArtifact {
            kind: "tysel-release-archive".into(),
            size_bytes: first.len() as u64,
            sha256: first_sha256.clone(),
        },
        builds: commands
            .iter()
            .enumerate()
            .map(|(index, command)| ReproducibleBuild {
                ordinal: (index + 1) as u8,
                command: command.clone(),
                artifact_sha256: first_sha256.clone(),
            })
            .collect(),
    })
}

pub fn write_reproducible_build_evidence(
    path: impl AsRef<Path>,
    evidence: &ReproducibleBuildEvidence,
) -> Result<PathBuf> {
    let path = path.as_ref();
    write_json_atomically(path, evidence)?;
    Ok(path.to_owned())
}

pub fn verify_reproducible_build_evidence(
    artifact: impl AsRef<Path>,
    evidence_path: impl AsRef<Path>,
    cargo_lock: impl AsRef<Path>,
    expected_target: &str,
) -> Result<ReproducibleBuildEvidence> {
    let evidence_bytes = read_evidence(evidence_path.as_ref())?;
    let evidence: ReproducibleBuildEvidence = serde_json::from_slice(&evidence_bytes)
        .with_context(|| format!("invalid JSON in {}", evidence_path.as_ref().display()))?;
    ensure!(
        evidence.evidence_version == REPRODUCIBLE_BUILD_EVIDENCE_VERSION,
        "unsupported reproducibility evidence version"
    );
    let commands = evidence.builds.iter().map(|build| build.command.clone()).collect::<Vec<_>>();
    validate_provenance(&evidence.source_commit, &evidence.target, &evidence.toolchain, &commands)?;
    ensure!(
        evidence.target == expected_target,
        "reproducibility evidence target does not match the expected deployment target"
    );
    ensure!(evidence.artifact.kind == "tysel-release-archive", "unexpected artifact kind");
    ensure!(
        evidence.builds[0].ordinal == 1 && evidence.builds[1].ordinal == 2,
        "reproducibility build ordinals must be 1 and 2"
    );
    ensure!(
        evidence.builds.iter().all(|build| build.artifact_sha256 == evidence.artifact.sha256),
        "reproducibility build digest does not match the artifact"
    );
    let artifact = read_artifact(artifact.as_ref())?;
    ensure!(
        artifact.len() as u64 == evidence.artifact.size_bytes,
        "artifact size does not match reproducibility evidence"
    );
    ensure!(
        bundle_hash(&artifact) == evidence.artifact.sha256,
        "artifact digest does not match reproducibility evidence"
    );

    let cargo_lock = fs::read(cargo_lock.as_ref())
        .with_context(|| format!("failed to read {}", cargo_lock.as_ref().display()))?;
    let cargo_lock_sha256 = bundle_hash(&cargo_lock);
    ensure!(
        cargo_lock_sha256 == evidence.cargo_lock_sha256,
        "Cargo.lock digest does not match reproducibility evidence"
    );
    let inventory = embedded_runtime_inventory()?;
    ensure!(
        cargo_lock_sha256 == inventory.cargo_lock_sha256,
        "Cargo.lock does not match the embedded runtime inventory"
    );
    ensure!(
        inventory_digest() == evidence.runtime_inventory_sha256,
        "runtime inventory digest does not match reproducibility evidence"
    );
    Ok(evidence)
}

fn read_artifact(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect release artifact {}", path.display()))?;
    ensure!(metadata.is_file(), "release artifact {} is not a file", path.display());
    ensure!(metadata.len() > 0, "release artifact {} is empty", path.display());
    ensure!(
        metadata.len() <= MAX_REPRODUCIBLE_ARTIFACT_BYTES,
        "release artifact {} exceeds {} bytes",
        path.display(),
        MAX_REPRODUCIBLE_ARTIFACT_BYTES
    );
    fs::read(path).with_context(|| format!("failed to read release artifact {}", path.display()))
}

fn read_evidence(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).with_context(|| {
        format!("failed to inspect reproducibility evidence {}", path.display())
    })?;
    ensure!(metadata.is_file(), "reproducibility evidence {} is not a file", path.display());
    ensure!(metadata.len() <= 1024 * 1024, "reproducibility evidence is oversized");
    fs::read(path)
        .with_context(|| format!("failed to read reproducibility evidence {}", path.display()))
}

fn validate_provenance(
    source_commit: &str,
    target: &str,
    toolchain: &str,
    commands: &[String],
) -> Result<()> {
    ensure!(
        valid_lower_hex(source_commit, &[40, 64]),
        "source commit must be 40 or 64 lowercase hex characters"
    );
    ensure!(matches!(target, "linux-x64" | "linux-arm64"), "unsupported production release target");
    validate_single_line("toolchain", toolchain)?;
    ensure!(commands.len() == 2, "reproducibility evidence requires exactly two build commands");
    for command in commands {
        validate_single_line("build command", command)?;
    }
    Ok(())
}

fn validate_single_line(label: &str, value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_PROVENANCE_VALUE_BYTES,
        "{label} must contain 1..={MAX_PROVENANCE_VALUE_BYTES} bytes"
    );
    ensure!(!value.contains(['\r', '\n']), "{label} must be a single line");
    Ok(())
}

fn valid_lower_hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_identical_artifacts_and_binds_the_locked_graph() {
        let root = temp_root("same");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.tar.gz");
        let second = root.join("second.tar.gz");
        fs::write(&first, b"deterministic archive").unwrap();
        fs::write(&second, b"deterministic archive").unwrap();
        let commands = vec!["build in target/a".into(), "build in target/b".into()];
        let evidence = compare_reproducible_builds(
            &first,
            &second,
            "0123456789abcdef0123456789abcdef01234567",
            "linux-x64",
            "rustc 1.97.1",
            &commands,
            workspace_root().join("Cargo.lock"),
        )
        .unwrap();

        assert_eq!(evidence.evidence_version, REPRODUCIBLE_BUILD_EVIDENCE_VERSION);
        assert_eq!(evidence.builds.len(), 2);
        assert_eq!(evidence.builds[0].artifact_sha256, evidence.artifact.sha256);
        assert_eq!(evidence.builds[1].artifact_sha256, evidence.artifact.sha256);
        let output = root.join("release.repro.json");
        write_reproducible_build_evidence(&output, &evidence).unwrap();
        let decoded = verify_reproducible_build_evidence(
            &first,
            &output,
            workspace_root().join("Cargo.lock"),
            "linux-x64",
        )
        .unwrap();
        assert_eq!(decoded, evidence);
        assert!(
            verify_reproducible_build_evidence(
                &first,
                &output,
                workspace_root().join("Cargo.lock"),
                "linux-arm64",
            )
            .unwrap_err()
            .to_string()
            .contains("expected deployment target")
        );

        let mut tampered: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        tampered["source_commit"] = serde_json::json!("main");
        fs::write(&output, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
        assert!(
            verify_reproducible_build_evidence(
                &first,
                &output,
                workspace_root().join("Cargo.lock"),
                "linux-x64",
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_different_artifacts_and_ambiguous_provenance() {
        let root = temp_root("different");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.tar.gz");
        let second = root.join("second.tar.gz");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let commands = vec!["build one".into(), "build two".into()];
        assert!(
            compare_reproducible_builds(
                &first,
                &second,
                "0123456789abcdef0123456789abcdef01234567",
                "linux-arm64",
                "rustc 1.97.1",
                &commands,
                workspace_root().join("Cargo.lock"),
            )
            .unwrap_err()
            .to_string()
            .contains("not reproducible")
        );
        assert!(validate_provenance("main", "linux-x64", "rustc 1.97.1", &commands).is_err());
        assert!(
            validate_provenance(
                "0123456789abcdef0123456789abcdef01234567",
                "darwin-arm64",
                "rustc 1.97.1",
                &commands
            )
            .is_err()
        );
    }

    #[test]
    fn schema_rejects_unknown_fields() {
        let mut value = serde_json::json!({
            "evidence_version": 1,
            "source_commit": "0123456789abcdef0123456789abcdef01234567",
            "target": "linux-x64",
            "toolchain": "rustc 1.97.1",
            "cargo_lock_sha256": "00".repeat(32),
            "runtime_inventory_sha256": "00".repeat(32),
            "artifact": {
                "kind": "tysel-release-archive",
                "size_bytes": 1,
                "sha256": "00".repeat(32)
            },
            "builds": []
        });
        value["future_security_flag"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReproducibleBuildEvidence>(value).is_err());
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tysel-reproducibility-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ))
    }
}
