use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use tysel_package::bundle_hash;
pub use tysel_release_signing::{
    RELEASE_ARTIFACT_SIGNATURE_VERSION, ReleaseArtifactSignature, sign_release_artifact,
};
use tysel_release_signing::{artifact_signature_message, validate_release_target};

use crate::evidence::{sidecar_path, verify_release_evidence, write_json_atomically};

pub const RELEASE_SIGNATURE_VERSION: u32 = 1;
pub const TRUST_POLICY_VERSION: u32 = 1;
const MAX_CLOCK_SKEW_SECONDS: u64 = 300;
const MAX_POLICY_LIFETIME_SECONDS: u64 = 90 * 24 * 60 * 60;
const MAX_SIGNING_DOCUMENT_BYTES: u64 = 1024 * 1024;
const MAX_SIGNED_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const SIGNATURE_DOMAIN: &[u8] = b"tysel-release-evidence-signature-v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSignature {
    pub signature_version: u32,
    pub algorithm: String,
    pub key_id: String,
    pub issued_at_unix: u64,
    pub evidence_sha256: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustPolicy {
    pub policy_version: u32,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub keys: Vec<TrustedReleaseKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedReleaseKey {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub status: ReleaseKeyStatus,
    pub valid_from_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until_unix: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKeyStatus {
    Active,
    Retired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseKeyInfo {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
}

pub fn release_key_info(private_key_path: impl AsRef<Path>) -> Result<ReleaseKeyInfo> {
    let signing_key = read_signing_key(private_key_path.as_ref())?;
    let public_key = signing_key.verifying_key().to_bytes();
    Ok(ReleaseKeyInfo {
        key_id: bundle_hash(&public_key),
        algorithm: "ed25519".into(),
        public_key: encode_hex(&public_key),
    })
}

pub fn sign_release_evidence(
    output: impl AsRef<Path>,
    private_key_path: impl AsRef<Path>,
    issued_at_unix: u64,
) -> Result<PathBuf> {
    let output = output.as_ref();
    verify_release_evidence(output)?;
    let evidence_path = sidecar_path(output, ".evidence.json");
    let evidence = read_bounded(&evidence_path)?;
    let evidence_sha256 = bundle_hash(&evidence);
    let signing_key = read_signing_key(private_key_path.as_ref())?;
    let public_key = signing_key.verifying_key().to_bytes();
    let key_id = bundle_hash(&public_key);
    let message = signature_message(&key_id, issued_at_unix, &evidence_sha256);
    let signature = signing_key.sign(&message);
    let document = ReleaseSignature {
        signature_version: RELEASE_SIGNATURE_VERSION,
        algorithm: "ed25519".into(),
        key_id,
        issued_at_unix,
        evidence_sha256,
        signature: encode_hex(&signature.to_bytes()),
    };
    let signature_path = sidecar_path(output, ".evidence.sig.json");
    write_json_atomically(&signature_path, &document)?;
    Ok(signature_path)
}

pub fn verify_release_signature(
    output: impl AsRef<Path>,
    trust_policy_path: impl AsRef<Path>,
    now_unix: u64,
) -> Result<ReleaseSignature> {
    let output = output.as_ref();
    verify_release_evidence(output)?;
    let signature_path = sidecar_path(output, ".evidence.sig.json");
    let signature_document: ReleaseSignature = read_json(&signature_path)?;
    ensure!(
        signature_document.signature_version == RELEASE_SIGNATURE_VERSION,
        "unsupported release signature version"
    );
    ensure!(signature_document.algorithm == "ed25519", "unsupported signature algorithm");
    ensure!(
        signature_document.issued_at_unix <= now_unix.saturating_add(MAX_CLOCK_SKEW_SECONDS),
        "release signature is from the future"
    );
    let evidence_path = sidecar_path(output, ".evidence.json");
    let evidence = read_bounded(&evidence_path)?;
    ensure!(
        bundle_hash(&evidence) == signature_document.evidence_sha256,
        "release signature does not bind this evidence index"
    );
    let policy: TrustPolicy = read_json(trust_policy_path.as_ref())?;
    validate_trust_policy(&policy)?;
    ensure!(
        policy.issued_at_unix <= now_unix.saturating_add(MAX_CLOCK_SKEW_SECONDS),
        "release trust policy is from the future"
    );
    ensure!(now_unix <= policy.expires_at_unix, "release trust policy has expired");
    ensure!(
        signature_document.issued_at_unix <= policy.expires_at_unix,
        "release signature postdates the trust policy"
    );
    let trusted = policy
        .keys
        .iter()
        .find(|key| key.key_id == signature_document.key_id)
        .context("release signature key is not trusted")?;
    ensure!(trusted.status != ReleaseKeyStatus::Revoked, "release signature key is revoked");
    ensure!(now_unix >= trusted.valid_from_unix, "release key is not active yet");
    if let Some(valid_until) = trusted.valid_until_unix {
        ensure!(now_unix <= valid_until, "release key validity has expired");
        ensure!(
            signature_document.issued_at_unix <= valid_until,
            "release signature postdates key validity"
        );
    }
    ensure!(
        signature_document.issued_at_unix >= trusted.valid_from_unix,
        "release signature predates key validity"
    );
    let public_key = VerifyingKey::from_bytes(&decode_hex::<32>(&trusted.public_key)?)
        .context("trusted Ed25519 public key is invalid")?;
    let signature = Signature::from_bytes(&decode_hex::<64>(&signature_document.signature)?);
    let message = signature_message(
        &signature_document.key_id,
        signature_document.issued_at_unix,
        &signature_document.evidence_sha256,
    );
    public_key
        .verify_strict(&message, &signature)
        .context("release evidence signature verification failed")?;
    Ok(signature_document)
}

pub fn verify_release_artifact_signature(
    artifact_path: impl AsRef<Path>,
    trust_policy_path: impl AsRef<Path>,
    expected_target: &str,
    now_unix: u64,
) -> Result<ReleaseArtifactSignature> {
    validate_release_target(expected_target)?;
    let artifact_path = artifact_path.as_ref();
    let document: ReleaseArtifactSignature = read_json(&sidecar_path(artifact_path, ".sig.json"))?;
    ensure!(
        document.signature_version == RELEASE_ARTIFACT_SIGNATURE_VERSION,
        "unsupported release artifact signature version"
    );
    ensure!(document.algorithm == "ed25519", "unsupported signature algorithm");
    validate_release_target(&document.target)?;
    ensure!(
        document.target == expected_target,
        "release artifact target does not match the expected deployment target"
    );
    ensure!(
        document.issued_at_unix <= now_unix.saturating_add(MAX_CLOCK_SKEW_SECONDS),
        "release artifact signature is from the future"
    );
    let artifact = read_bounded_artifact(artifact_path)?;
    ensure!(
        bundle_hash(&artifact) == document.artifact_sha256,
        "release artifact signature does not bind this artifact"
    );
    let policy: TrustPolicy = read_json(trust_policy_path.as_ref())?;
    let trusted =
        trusted_release_key(&policy, &document.key_id, document.issued_at_unix, now_unix)?;
    let public_key = VerifyingKey::from_bytes(&decode_hex::<32>(&trusted.public_key)?)
        .context("trusted Ed25519 public key is invalid")?;
    let signature = Signature::from_bytes(&decode_hex::<64>(&document.signature)?);
    let message = artifact_signature_message(
        &document.key_id,
        document.issued_at_unix,
        &document.target,
        &document.artifact_sha256,
    );
    public_key
        .verify_strict(&message, &signature)
        .context("release artifact signature verification failed")?;
    Ok(document)
}

pub fn validate_trust_policy(policy: &TrustPolicy) -> Result<()> {
    ensure!(policy.policy_version == TRUST_POLICY_VERSION, "unsupported trust policy version");
    ensure!(policy.expires_at_unix > policy.issued_at_unix, "trust policy validity is empty");
    ensure!(
        policy.expires_at_unix - policy.issued_at_unix <= MAX_POLICY_LIFETIME_SECONDS,
        "trust policy validity exceeds 90 days"
    );
    ensure!(!policy.keys.is_empty(), "trust policy contains no keys");
    let mut previous = None;
    for key in &policy.keys {
        ensure!(key.algorithm == "ed25519", "unsupported trusted-key algorithm");
        if let Some(valid_until) = key.valid_until_unix {
            ensure!(valid_until >= key.valid_from_unix, "trusted-key validity window is inverted");
        }
        ensure!(
            key.status != ReleaseKeyStatus::Retired || key.valid_until_unix.is_some(),
            "retired release keys require valid_until_unix"
        );
        let public_key = decode_hex::<32>(&key.public_key)?;
        ensure!(bundle_hash(&public_key) == key.key_id, "trusted key ID does not match public key");
        if let Some(previous) = previous {
            ensure!(previous < key.key_id.as_str(), "trusted keys are duplicated or unsorted");
        }
        previous = Some(key.key_id.as_str());
    }
    Ok(())
}

fn read_signing_key(path: &Path) -> Result<SigningKey> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open private key {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect private key {}", path.display()))?;
    ensure!(metadata.is_file(), "release private key is not a regular file");
    ensure!(metadata.len() <= 256, "release private key file is oversized");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "release private key permissions must deny group and other access"
        );
    }
    let mut encoded = SecretBuffer(Vec::with_capacity(metadata.len() as usize));
    file.read_to_end(&mut encoded.0)
        .with_context(|| format!("failed to read private key {}", path.display()))?;
    let encoded = std::str::from_utf8(&encoded.0).context("release private key is not UTF-8")?;
    Ok(SigningKey::from_bytes(&decode_hex::<32>(encoded.trim())?))
}

struct SecretBuffer(Vec<u8>);

impl Drop for SecretBuffer {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

fn signature_message(key_id: &str, issued_at_unix: u64, evidence_sha256: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(160);
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(key_id.as_bytes());
    message.push(0);
    message.extend_from_slice(issued_at_unix.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(evidence_sha256.as_bytes());
    message
}

fn trusted_release_key<'a>(
    policy: &'a TrustPolicy,
    key_id: &str,
    issued_at_unix: u64,
    now_unix: u64,
) -> Result<&'a TrustedReleaseKey> {
    validate_trust_policy(policy)?;
    ensure!(
        policy.issued_at_unix <= now_unix.saturating_add(MAX_CLOCK_SKEW_SECONDS),
        "release trust policy is from the future"
    );
    ensure!(now_unix <= policy.expires_at_unix, "release trust policy has expired");
    ensure!(
        issued_at_unix <= policy.expires_at_unix,
        "release signature postdates the trust policy"
    );
    let trusted = policy
        .keys
        .iter()
        .find(|key| key.key_id == key_id)
        .context("release signature key is not trusted")?;
    ensure!(trusted.status != ReleaseKeyStatus::Revoked, "release signature key is revoked");
    ensure!(now_unix >= trusted.valid_from_unix, "release key is not active yet");
    if let Some(valid_until) = trusted.valid_until_unix {
        ensure!(now_unix <= valid_until, "release key validity has expired");
        ensure!(issued_at_unix <= valid_until, "release signature postdates key validity");
    }
    ensure!(issued_at_unix >= trusted.valid_from_unix, "release signature predates key validity");
    Ok(trusted)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = read_bounded(path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    ensure!(
        metadata.len() <= MAX_SIGNING_DOCUMENT_BYTES,
        "signing document {} is oversized",
        path.display()
    );
    fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

fn read_bounded_artifact(path: &Path) -> Result<Vec<u8>> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    ensure!(metadata.is_file(), "release artifact is not a regular file");
    ensure!(metadata.len() > 0, "release artifact is empty");
    ensure!(metadata.len() <= MAX_SIGNED_ARTIFACT_BYTES, "release artifact is oversized");
    fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex<const N: usize>(encoded: &str) -> Result<[u8; N]> {
    ensure!(encoded.len() == N * 2, "hex value has the wrong length");
    let mut decoded = [0; N];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => anyhow::bail!("hex values must use canonical lowercase encoding"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_package::{PackageManifest, Tap};

    fn release_stub() -> Vec<u8> {
        let mut stub = b"release-binary".to_vec();
        stub.extend_from_slice(crate::supply_chain::embedded_runtime_inventory_bytes());
        stub
    }

    #[test]
    fn signs_verifies_retires_and_revokes_release_evidence() {
        let root = temp_root("rotation");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(&release_stub()).unwrap()).unwrap();
        crate::write_release_evidence(&output, "linux-x64").unwrap();
        let key_path = write_key(&root, [7; 32]);
        let key_info = release_key_info(&key_path).unwrap();
        let trust_path = root.join("trust.json");
        let mut policy = TrustPolicy {
            policy_version: TRUST_POLICY_VERSION,
            issued_at_unix: 0,
            expires_at_unix: 10_000,
            keys: vec![TrustedReleaseKey {
                key_id: key_info.key_id,
                algorithm: key_info.algorithm,
                public_key: key_info.public_key,
                status: ReleaseKeyStatus::Active,
                valid_from_unix: 900,
                valid_until_unix: None,
            }],
        };
        write_policy(&trust_path, &policy);
        sign_release_evidence(&output, &key_path, 1_000).unwrap();
        assert_eq!(
            verify_release_signature(&output, &trust_path, 1_000).unwrap().issued_at_unix,
            1_000
        );

        policy.keys[0].status = ReleaseKeyStatus::Retired;
        policy.keys[0].valid_until_unix = Some(2_000);
        write_policy(&trust_path, &policy);
        verify_release_signature(&output, &trust_path, 2_000).unwrap();
        let error = verify_release_signature(&output, &trust_path, 2_001).unwrap_err();
        assert!(error.to_string().contains("validity has expired"));

        policy.keys[0].status = ReleaseKeyStatus::Revoked;
        write_policy(&trust_path, &policy);
        let error = verify_release_signature(&output, &trust_path, 2_000).unwrap_err();
        assert!(error.to_string().contains("revoked"));
    }

    #[test]
    fn rejects_tampered_signatures_future_dates_and_invalid_policies() {
        let root = temp_root("reject");
        fs::create_dir_all(&root).unwrap();
        let output = root.join("release-app");
        fs::write(&output, tap().embed_into(&release_stub()).unwrap()).unwrap();
        crate::write_release_evidence(&output, "linux-x64").unwrap();
        let key_path = write_key(&root, [9; 32]);
        let key_info = release_key_info(&key_path).unwrap();
        let trust_path = root.join("trust.json");
        let mut policy = TrustPolicy {
            policy_version: TRUST_POLICY_VERSION,
            issued_at_unix: 0,
            expires_at_unix: 20_000,
            keys: vec![TrustedReleaseKey {
                key_id: key_info.key_id,
                algorithm: key_info.algorithm,
                public_key: key_info.public_key,
                status: ReleaseKeyStatus::Active,
                valid_from_unix: 0,
                valid_until_unix: None,
            }],
        };
        write_policy(&trust_path, &policy);
        sign_release_evidence(&output, &key_path, 10_000).unwrap();
        let error = verify_release_signature(&output, &trust_path, 9_000).unwrap_err();
        assert!(error.to_string().contains("future"));

        sign_release_evidence(&output, &key_path, 9_000).unwrap();
        let signature_path = sidecar_path(&output, ".evidence.sig.json");
        let mut document: ReleaseSignature = read_json(&signature_path).unwrap();
        document.signature.replace_range(0..2, "00");
        write_json_atomically(&signature_path, &document).unwrap();
        let error = verify_release_signature(&output, &trust_path, 9_000).unwrap_err();
        assert!(error.to_string().contains("verification failed"));

        policy.keys[0].key_id = "00".repeat(32);
        assert!(validate_trust_policy(&policy).unwrap_err().to_string().contains("does not match"));

        policy.keys[0].key_id = bundle_hash(&decode_hex::<32>(&policy.keys[0].public_key).unwrap());
        policy.expires_at_unix = MAX_POLICY_LIFETIME_SECONDS + 1;
        assert!(validate_trust_policy(&policy).unwrap_err().to_string().contains("90 days"));
    }

    #[test]
    fn signs_and_verifies_distribution_artifacts() {
        let root = temp_root("artifact");
        fs::create_dir_all(&root).unwrap();
        let artifact = root.join("tysel-linux-x64.tar.gz");
        fs::write(&artifact, b"deterministic release archive").unwrap();
        let key_path = write_key(&root, [11; 32]);
        let key_info = release_key_info(&key_path).unwrap();
        let trust_path = root.join("trust.json");
        write_policy(
            &trust_path,
            &TrustPolicy {
                policy_version: TRUST_POLICY_VERSION,
                issued_at_unix: 0,
                expires_at_unix: 10_000,
                keys: vec![TrustedReleaseKey {
                    key_id: key_info.key_id,
                    algorithm: key_info.algorithm,
                    public_key: key_info.public_key,
                    status: ReleaseKeyStatus::Active,
                    valid_from_unix: 0,
                    valid_until_unix: None,
                }],
            },
        );

        sign_release_artifact(&artifact, "linux-x64", &key_path, 1_000).unwrap();
        let signature =
            verify_release_artifact_signature(&artifact, &trust_path, "linux-x64", 1_000).unwrap();
        assert_eq!(signature.target, "linux-x64");
        assert!(
            verify_release_artifact_signature(&artifact, &trust_path, "linux-arm64", 1_000)
                .unwrap_err()
                .to_string()
                .contains("expected deployment target")
        );
        fs::write(&artifact, b"tampered archive").unwrap();
        assert!(
            verify_release_artifact_signature(&artifact, &trust_path, "linux-x64", 1_000)
                .unwrap_err()
                .to_string()
                .contains("does not bind")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_private_keys_readable_by_group_or_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("permissions");
        fs::create_dir_all(&root).unwrap();
        let key_path = write_key(&root, [3; 32]);
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = release_key_info(&key_path).unwrap_err();
        assert!(error.to_string().contains("permissions"));
    }

    fn tap() -> Tap {
        Tap::new(
            PackageManifest {
                format_version: 0,
                runtime_version: "1.0.0".into(),
                application_id: "signed-app".into(),
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

    fn write_key(root: &Path, seed: [u8; 32]) -> PathBuf {
        let path = root.join("release.key");
        fs::write(&path, format!("{}\n", encode_hex(&seed))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    fn write_policy(path: &Path, policy: &TrustPolicy) {
        write_json_atomically(path, policy).unwrap();
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tysel-release-signing-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ))
    }
}
