use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tysel_package::{Tap, TapCompatibilityReport, bundle_hash, compatibility_report};

use crate::supply_chain::{
    CycloneDxBom, LicenseInventory, embedded_runtime_inventory_bytes, inventory_digest,
    release_supply_chain,
};

pub const RELEASE_EVIDENCE_VERSION: u32 = 2;
const MAX_RELEASE_SIDECAR_BYTES: u64 = 32 * 1024 * 1024;
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
    pub supply_chain: ReleaseSupplyChainEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseArtifactEvidence {
    pub kind: String,
    pub target: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSupplyChainEvidence {
    pub runtime_inventory_sha256: String,
    pub sbom: ReleaseDocumentEvidence,
    pub licenses: ReleaseDocumentEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseDocumentEvidence {
    pub kind: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseSidecars {
    pub checksum: PathBuf,
    pub compatibility: PathBuf,
    pub sbom: PathBuf,
    pub licenses: PathBuf,
    pub evidence: PathBuf,
}

pub fn write_release_evidence(output: impl AsRef<Path>, target: &str) -> Result<ReleaseSidecars> {
    let output = output.as_ref();
    let sidecars = ReleaseSidecars {
        checksum: sidecar_path(output, ".sha256"),
        compatibility: sidecar_path(output, ".compat.json"),
        sbom: sidecar_path(output, ".sbom.cdx.json"),
        licenses: sidecar_path(output, ".licenses.json"),
        evidence: sidecar_path(output, ".evidence.json"),
    };
    remove_file_if_present(&sidecars.evidence)?;
    let artifact = fs::read(output)
        .with_context(|| format!("failed to read release artifact {}", output.display()))?;
    let tap = Tap::from_path(output).with_context(|| {
        format!("release artifact {} contains an invalid TAP", output.display())
    })?;
    ensure_release_runtime_identity(&artifact)?;
    let tap_payload = tap.encode().context("failed to encode embedded TAP for release evidence")?;
    let compatibility = compatibility_report(&tap_payload);
    ensure!(compatibility.compatible, "release TAP is not compatible with this runtime");
    let digest = bundle_hash(&artifact);
    let (sbom, licenses) = release_supply_chain(&tap, &digest)?;
    let sbom_bytes = json_bytes(&sbom)?;
    let license_bytes = json_bytes(&licenses)?;
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
        supply_chain: ReleaseSupplyChainEvidence {
            runtime_inventory_sha256: inventory_digest(),
            sbom: document_evidence("cyclonedx-1.5", &sbom_bytes),
            licenses: document_evidence("spdx-license-inventory", &license_bytes),
        },
    };
    let compatibility = stage_json(&sidecars.compatibility, &compatibility)?;
    let sbom = StagedFile::new(&sidecars.sbom, &sbom_bytes)?;
    let licenses = StagedFile::new(&sidecars.licenses, &license_bytes)?;
    let evidence = stage_json(&sidecars.evidence, &index)?;
    let checksum = StagedFile::new(&sidecars.checksum, format!("{digest}\n").as_bytes())?;

    compatibility.commit()?;
    sbom.commit()?;
    licenses.commit()?;
    checksum.commit()?;
    evidence.commit()?;
    Ok(sidecars)
}

pub fn verify_release_evidence(output: impl AsRef<Path>) -> Result<ReleaseEvidenceIndex> {
    let output = output.as_ref();
    let index: ReleaseEvidenceIndex = read_json(&sidecar_path(output, ".evidence.json"))?;
    ensure!(
        index.evidence_version == RELEASE_EVIDENCE_VERSION,
        "unsupported release evidence version"
    );
    let artifact = fs::read(output)
        .with_context(|| format!("failed to read release artifact {}", output.display()))?;
    ensure!(
        artifact.len() as u64 == index.artifact.size_bytes,
        "release artifact size does not match evidence"
    );
    ensure!(
        bundle_hash(&artifact) == index.artifact.sha256,
        "release artifact digest does not match evidence"
    );
    ensure!(index.artifact.kind == "tysel-single-executable", "unexpected artifact kind");
    ensure!(!index.artifact.target.is_empty(), "release target is empty");
    let tap = Tap::from_path(output).context("release artifact contains an invalid TAP")?;
    ensure_release_runtime_identity(&artifact)?;
    ensure!(
        tap.manifest.application_id == index.application_id,
        "application identity does not match evidence"
    );
    ensure!(
        tap.manifest.execution_profile == index.execution_profile,
        "execution profile does not match evidence"
    );
    let tap_payload = tap.encode().context("failed to encode embedded TAP during verification")?;
    ensure!(
        compatibility_report(&tap_payload) == index.compatibility,
        "embedded TAP compatibility does not match evidence"
    );
    let checksum_path = sidecar_path(output, ".sha256");
    let checksum_bytes = read_bounded(&checksum_path, MAX_RELEASE_SIDECAR_BYTES)?;
    let checksum = std::str::from_utf8(&checksum_bytes).context("checksum sidecar is not UTF-8")?;
    ensure!(
        checksum == format!("{}\n", index.artifact.sha256),
        "checksum sidecar does not match evidence"
    );
    let compatibility: TapCompatibilityReport = read_json(&sidecar_path(output, ".compat.json"))?;
    ensure!(compatibility == index.compatibility, "compatibility sidecar does not match evidence");
    let sbom_bytes = verify_document(output, ".sbom.cdx.json", &index.supply_chain.sbom)?;
    let license_bytes = verify_document(output, ".licenses.json", &index.supply_chain.licenses)?;
    let sbom: CycloneDxBom = serde_json::from_slice(&sbom_bytes)?;
    let licenses: LicenseInventory = serde_json::from_slice(&license_bytes)?;
    let (expected_sbom, expected_licenses) = release_supply_chain(&tap, &index.artifact.sha256)?;
    ensure!(sbom == expected_sbom, "SBOM does not match the embedded TAP and runtime inventory");
    ensure!(
        licenses == expected_licenses,
        "license inventory does not match the embedded runtime inventory"
    );
    ensure!(
        sbom.metadata
            .component
            .hashes
            .iter()
            .any(|hash| { hash.alg == "SHA-256" && hash.content == index.artifact.sha256 }),
        "SBOM does not identify the release artifact"
    );
    ensure!(
        sbom.components.len() == licenses.components.len(),
        "SBOM and license inventory component counts differ"
    );
    ensure!(
        sbom.components.iter().zip(&licenses.components).all(|(component, licensed)| {
            component.name == licensed.name
                && component.version.as_deref() == Some(licensed.version.as_str())
                && component.purl.as_deref() == Some(licensed.purl.as_str())
                && component.licenses.as_slice()
                    == [crate::supply_chain::BomLicenseChoice {
                        expression: licensed.license.clone(),
                    }]
        }),
        "SBOM and license inventory components differ"
    );
    ensure!(
        index.supply_chain.runtime_inventory_sha256 == inventory_digest(),
        "embedded runtime inventory does not match evidence"
    );
    Ok(index)
}

fn verify_document(
    output: &Path,
    suffix: &str,
    evidence: &ReleaseDocumentEvidence,
) -> Result<Vec<u8>> {
    let path = sidecar_path(output, suffix);
    let bytes = read_bounded(&path, MAX_RELEASE_SIDECAR_BYTES)?;
    ensure!(
        bytes.len() as u64 == evidence.size_bytes,
        "{} size does not match evidence",
        evidence.kind
    );
    ensure!(
        bundle_hash(&bytes) == evidence.sha256,
        "{} digest does not match evidence",
        evidence.kind
    );
    Ok(bytes)
}

fn document_evidence(kind: &str, contents: &[u8]) -> ReleaseDocumentEvidence {
    ReleaseDocumentEvidence {
        kind: kind.into(),
        size_bytes: contents.len() as u64,
        sha256: bundle_hash(contents),
    }
}

fn ensure_release_runtime_identity(artifact: &[u8]) -> Result<()> {
    ensure!(artifact.len() >= 16, "release artifact contains no TAP footer");
    let footer = artifact.len() - 16;
    ensure!(&artifact[footer + 8..] == b"TYSELEND", "release artifact contains no TAP footer");
    let payload_len = u64::from_le_bytes(
        artifact[footer..footer + 8].try_into().expect("eight-byte TAP payload length"),
    );
    let payload_len =
        usize::try_from(payload_len).context("TAP payload length is not addressable")?;
    ensure!(payload_len <= footer, "TAP payload length exceeds release artifact");
    let stub = &artifact[..footer - payload_len];
    let inventory = embedded_runtime_inventory_bytes();
    ensure!(
        stub.windows(inventory.len()).any(|window| window == inventory),
        "release runtime stub does not embed the current locked runtime inventory"
    );
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = read_bounded(path, MAX_RELEASE_SIDECAR_BYTES)?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    ensure!(metadata.len() <= max_bytes, "release sidecar {} is oversized", path.display());
    fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

pub(crate) fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    stage_json(path, value)?.commit()
}

fn stage_json(path: &Path, value: &impl Serialize) -> Result<StagedFile> {
    StagedFile::new(path, &json_bytes(value)?)
}

fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    Ok(json)
}

pub(crate) fn sidecar_path(output: &Path, suffix: &str) -> PathBuf {
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
                http1: true,
                http2: false,
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

    fn release_stub() -> Vec<u8> {
        let mut stub = b"release-binary".to_vec();
        stub.extend_from_slice(embedded_runtime_inventory_bytes());
        stub
    }

    #[test]
    fn writes_deterministic_release_sidecars() {
        let root = temp_root("deterministic");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        let artifact = tap().embed_into(&release_stub()).unwrap();
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
        assert_eq!(index.supply_chain.sbom.kind, "cyclonedx-1.5");
        assert!(first.sbom.exists());
        assert!(first.licenses.exists());
        assert_eq!(verify_release_evidence(&output).unwrap(), index);
    }

    #[test]
    fn verification_rejects_tampered_supply_chain_evidence() {
        let root = temp_root("tampered-sbom");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(&release_stub()).unwrap()).unwrap();
        let sidecars = write_release_evidence(&output, "linux-x64").unwrap();
        fs::write(&sidecars.sbom, b"{}\n").unwrap();

        let error = verify_release_evidence(&output).unwrap_err();
        assert!(error.to_string().contains("size does not match evidence"));
    }

    #[test]
    fn verification_rejects_an_index_that_misidentifies_the_artifact() {
        let root = temp_root("wrong-identity");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(&release_stub()).unwrap()).unwrap();
        let sidecars = write_release_evidence(&output, "linux-x64").unwrap();
        let mut index: ReleaseEvidenceIndex = read_json(&sidecars.evidence).unwrap();
        index.application_id = "different-application".into();
        fs::write(&sidecars.evidence, json_bytes(&index).unwrap()).unwrap();

        let error = verify_release_evidence(&output).unwrap_err();
        assert!(error.to_string().contains("application identity does not match evidence"));
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
    fn rejects_a_stale_or_unidentified_release_runtime_stub() {
        let root = temp_root("stale-runtime");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(b"stale-runtime-binary").unwrap()).unwrap();

        let error = write_release_evidence(&output, "linux-x64").unwrap_err();
        assert!(error.to_string().contains("current locked runtime inventory"));
        assert!(!sidecar_path(&output, ".evidence.json").exists());
    }

    #[test]
    fn failed_sidecar_publish_removes_the_evidence_commit_marker() {
        let root = temp_root("publish-failure");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(&release_stub()).unwrap()).unwrap();
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
