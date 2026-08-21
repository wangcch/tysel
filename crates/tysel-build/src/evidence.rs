use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tysel_package::{Tap, TapCompatibilityReport, bundle_hash, compatibility_report};

pub const RELEASE_EVIDENCE_VERSION: u32 = 1;
static TEMP_FILE_IDS: AtomicU64 = AtomicU64::new(1);

/// Deterministic index tying one executable digest to its TAP compatibility
/// decision. Timestamped attestations and signatures can reference this file
/// without changing its reproducible contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseEvidenceIndex {
    pub evidence_version: u32,
    pub artifact: ReleaseArtifactEvidence,
    pub application_id: String,
    pub execution_profile: String,
    pub compatibility: TapCompatibilityReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseArtifactEvidence {
    pub kind: String,
    pub target: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseSidecars {
    pub checksum: PathBuf,
    pub compatibility: PathBuf,
    pub evidence: PathBuf,
}

pub fn write_release_evidence(output: impl AsRef<Path>, target: &str) -> Result<ReleaseSidecars> {
    let output = output.as_ref();
    let sidecars = ReleaseSidecars {
        checksum: sidecar_path(output, ".sha256"),
        compatibility: sidecar_path(output, ".compat.json"),
        evidence: sidecar_path(output, ".evidence.json"),
    };
    remove_file_if_present(&sidecars.evidence)?;
    let artifact = fs::read(output)
        .with_context(|| format!("failed to read release artifact {}", output.display()))?;
    let tap = Tap::from_path(output).with_context(|| {
        format!("release artifact {} contains an invalid TAP", output.display())
    })?;
    let tap_payload = tap.encode().context("failed to encode embedded TAP for release evidence")?;
    let compatibility = compatibility_report(&tap_payload);
    ensure!(compatibility.compatible, "release TAP is not compatible with this runtime");
    let digest = bundle_hash(&artifact);
    let index = ReleaseEvidenceIndex {
        evidence_version: RELEASE_EVIDENCE_VERSION,
        artifact: ReleaseArtifactEvidence {
            kind: "tysel-single-executable".into(),
            target: target.to_owned(),
            size_bytes: artifact.len() as u64,
            sha256: digest.clone(),
        },
        application_id: tap.manifest.application_id.clone(),
        execution_profile: tap.manifest.execution_profile.clone(),
        compatibility: compatibility.clone(),
    };
    let compatibility = stage_json(&sidecars.compatibility, &compatibility)?;
    let evidence = stage_json(&sidecars.evidence, &index)?;
    let checksum = StagedFile::new(&sidecars.checksum, format!("{digest}\n").as_bytes())?;

    compatibility.commit()?;
    checksum.commit()?;
    evidence.commit()?;
    Ok(sidecars)
}

fn stage_json(path: &Path, value: &impl Serialize) -> Result<StagedFile> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    StagedFile::new(path, &json)
}

fn sidecar_path(output: &Path, suffix: &str) -> PathBuf {
    let mut path = OsString::from(output.as_os_str());
    path.push(suffix);
    PathBuf::from(path)
}

struct StagedFile {
    temporary: Option<PathBuf>,
    destination: PathBuf,
}

impl StagedFile {
    fn new(destination: &Path, contents: &[u8]) -> Result<Self> {
        for _ in 0..16 {
            let id = TEMP_FILE_IDS.fetch_add(1, Ordering::Relaxed);
            let temporary = sidecar_path(destination, &format!(".tmp-{}-{id}", std::process::id()));
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&temporary) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to stage release evidence {}", destination.display())
                    });
                }
            };
            let staged = Self { temporary: Some(temporary), destination: destination.to_owned() };
            file.write_all(contents)?;
            file.sync_all()?;
            return Ok(staged);
        }
        anyhow::bail!("failed to allocate temporary release evidence file")
    }

    fn commit(mut self) -> Result<()> {
        let temporary = self.temporary.as_ref().expect("staged file path");
        #[cfg(windows)]
        remove_file_if_present(&self.destination)?;
        fs::rename(temporary, &self.destination).with_context(|| {
            format!("failed to publish release evidence {}", self.destination.display())
        })?;
        self.temporary = None;
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if let Some(path) = self.temporary.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to replace release evidence {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_package::PackageManifest;

    fn tap() -> Tap {
        Tap::new(
            PackageManifest {
                format_version: 0,
                runtime_version: "1.0.0".into(),
                application_id: "release-app".into(),
                entrypoint: "src/index.js".into(),
                execution_profile: "service".into(),
                listen: "127.0.0.1:3000".into(),
                memory_limit_bytes: 128 * 1024 * 1024,
                cpu_ms_per_turn: 50,
                request_timeout_ms: 30_000,
                bundle_hash: String::new(),
                max_request_bytes: 16 * 1024 * 1024,
                websocket: false,
                sqlite_path: String::new(),
                secret_names: Vec::new(),
                fetch_hosts: Vec::new(),
                postgres: Vec::new(),
                fs_read: Vec::new(),
                fs_write: Vec::new(),
                json_logs: true,
            },
            b"export default {};".to_vec(),
            Vec::new(),
        )
    }

    #[test]
    fn writes_deterministic_release_sidecars() {
        let root = temp_root("deterministic");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        let artifact = tap().embed_into(b"release-binary").unwrap();
        fs::write(&output, &artifact).unwrap();

        let first = write_release_evidence(&output, "linux-x64").unwrap();
        let first_evidence = fs::read(&first.evidence).unwrap();
        let first_compatibility = fs::read(&first.compatibility).unwrap();
        let second = write_release_evidence(&output, "linux-x64").unwrap();
        assert_eq!(first_evidence, fs::read(&second.evidence).unwrap());
        assert_eq!(first_compatibility, fs::read(&second.compatibility).unwrap());
        assert_eq!(
            fs::read_to_string(&first.checksum).unwrap(),
            format!("{}\n", bundle_hash(&artifact))
        );

        let index: ReleaseEvidenceIndex = serde_json::from_slice(&first_evidence).unwrap();
        assert_eq!(index.evidence_version, RELEASE_EVIDENCE_VERSION);
        assert_eq!(index.artifact.target, "linux-x64");
        assert_eq!(index.artifact.size_bytes, artifact.len() as u64);
        assert!(index.compatibility.compatible);
        assert_eq!(index.application_id, "release-app");
    }

    #[test]
    fn rejects_artifacts_without_the_tap_the_evidence_would_describe() {
        let root = temp_root("invalid-artifact");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, b"not-a-packaged-executable").unwrap();
        fs::write(sidecar_path(&output, ".evidence.json"), b"stale-evidence").unwrap();

        let error = write_release_evidence(&output, "linux-x64").unwrap_err();
        assert!(error.to_string().contains("invalid TAP"));
        assert!(!sidecar_path(&output, ".evidence.json").exists());
        assert!(!sidecar_path(&output, ".compat.json").exists());
        assert!(!sidecar_path(&output, ".sha256").exists());
    }

    #[test]
    fn failed_sidecar_publish_removes_the_evidence_commit_marker() {
        let root = temp_root("publish-failure");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(b"release-binary").unwrap()).unwrap();
        let evidence = sidecar_path(&output, ".evidence.json");
        let checksum = sidecar_path(&output, ".sha256");
        fs::write(&evidence, b"stale-evidence").unwrap();
        fs::create_dir(&checksum).unwrap();

        assert!(write_release_evidence(&output, "linux-x64").is_err());
        assert!(!evidence.exists(), "failed publication must not leave a commit marker");
    }

    #[test]
    fn evidence_schema_rejects_unknown_fields() {
        let artifact = ReleaseArtifactEvidence {
            kind: "tysel-single-executable".into(),
            target: "linux-x64".into(),
            size_bytes: 1,
            sha256: "00".repeat(32),
        };
        let mut value = serde_json::to_value(&artifact).unwrap();
        value["futureSecurityFlag"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReleaseArtifactEvidence>(value).is_err());
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tysel-release-evidence-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ))
    }
}
