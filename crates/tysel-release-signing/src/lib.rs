use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RELEASE_ARTIFACT_SIGNATURE_VERSION: u32 = 1;
const MAX_SIGNED_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const ARTIFACT_SIGNATURE_DOMAIN: &[u8] = b"tysel-release-artifact-signature-v1\0";
static TEMP_FILE_IDS: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseArtifactSignature {
    pub signature_version: u32,
    pub algorithm: String,
    pub key_id: String,
    pub issued_at_unix: u64,
    pub target: String,
    pub artifact_sha256: String,
    pub signature: String,
}

pub fn sign_release_artifact(
    artifact_path: impl AsRef<Path>,
    target: &str,
    private_key_path: impl AsRef<Path>,
    issued_at_unix: u64,
) -> Result<PathBuf> {
    validate_release_target(target)?;
    let artifact_path = artifact_path.as_ref();
    let artifact_sha256 = hash_file(artifact_path)?;
    let signing_key = read_signing_key(private_key_path.as_ref())?;
    let public_key = signing_key.verifying_key().to_bytes();
    let key_id = encode_hex(&Sha256::digest(public_key));
    let message = artifact_signature_message(&key_id, issued_at_unix, target, &artifact_sha256);
    let signature = signing_key.sign(&message);
    let document = ReleaseArtifactSignature {
        signature_version: RELEASE_ARTIFACT_SIGNATURE_VERSION,
        algorithm: "ed25519".into(),
        key_id,
        issued_at_unix,
        target: target.into(),
        artifact_sha256,
        signature: encode_hex(&signature.to_bytes()),
    };
    let signature_path = sidecar_path(artifact_path, ".sig.json");
    write_json_atomically(&signature_path, &document)?;
    Ok(signature_path)
}

pub fn validate_release_target(target: &str) -> Result<()> {
    ensure!(matches!(target, "linux-x64" | "linux-arm64"), "unsupported production release target");
    Ok(())
}

#[doc(hidden)]
pub fn artifact_signature_message(
    key_id: &str,
    issued_at_unix: u64,
    target: &str,
    artifact_sha256: &str,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(192);
    message.extend_from_slice(ARTIFACT_SIGNATURE_DOMAIN);
    message.extend_from_slice(key_id.as_bytes());
    message.push(0);
    message.extend_from_slice(issued_at_unix.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(target.as_bytes());
    message.push(0);
    message.extend_from_slice(artifact_sha256.as_bytes());
    message
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open release artifact {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect release artifact {}", path.display()))?;
    ensure!(metadata.is_file(), "release artifact is not a regular file");
    ensure!(metadata.len() > 0, "release artifact is empty");
    ensure!(metadata.len() <= MAX_SIGNED_ARTIFACT_BYTES, "release artifact is oversized");
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read release artifact {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(encode_hex(&hasher.finalize()))
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

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');
    for _ in 0..16 {
        let id = TEMP_FILE_IDS.fetch_add(1, Ordering::Relaxed);
        let temporary = sidecar_path(path, &format!(".tmp-{}-{id}", std::process::id()));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to stage release signature {}", path.display())
                });
            }
        };
        let result = (|| -> Result<()> {
            file.write_all(&contents)?;
            file.sync_all()?;
            drop(file);
            #[cfg(windows)]
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            fs::rename(&temporary, path).with_context(|| {
                format!("failed to publish release signature {}", path.display())
            })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    anyhow::bail!("failed to allocate temporary release signature file")
}

fn sidecar_path(output: &Path, suffix: &str) -> PathBuf {
    let mut path = OsString::from(output.as_os_str());
    path.push(suffix);
    PathBuf::from(path)
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
        _ => anyhow::bail!("hex value must be lowercase"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let id = TEMP_FILE_IDS.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("tysel-release-signing-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let artifact = root.join("release.tar.gz");
        let key = root.join("release.key");
        fs::write(&artifact, b"deterministic release archive").unwrap();
        fs::write(&key, "07".repeat(32)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        }
        (root, artifact, key)
    }

    #[test]
    fn signs_only_supported_targets_with_restricted_keys() {
        let (root, artifact, key) = fixture("artifact");
        let signature_path = sign_release_artifact(&artifact, "linux-x64", &key, 1_000).unwrap();
        let signature: ReleaseArtifactSignature =
            serde_json::from_slice(&fs::read(signature_path).unwrap()).unwrap();
        assert_eq!(signature.target, "linux-x64");
        assert_eq!(signature.issued_at_unix, 1_000);
        assert_eq!(
            signature.artifact_sha256,
            encode_hex(&Sha256::digest(fs::read(&artifact).unwrap()))
        );
        assert!(sign_release_artifact(&artifact, "darwin-arm64", &key, 1_000).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
            let error = sign_release_artifact(&artifact, "linux-x64", &key, 1_000).unwrap_err();
            assert!(error.to_string().contains("permissions"));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
