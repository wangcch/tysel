use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sourcemap::SourceMap;

pub const TAP_VERSION: u32 = 1;

const TAP_MAGIC: &[u8; 8] = b"TYSELTAP";
const END_MAGIC: &[u8; 8] = b"TYSELEND";

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid tap: {0}")]
    Invalid(String),
    #[error("executable has no tap payload")]
    MissingPayload,
    #[error("unsupported tap version {0}")]
    Version(u32),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub format_version: u32,
    pub runtime_version: String,
    pub application_id: String,
    pub entrypoint: String,
    pub execution_profile: String,
    pub listen: String,
    pub memory_limit_bytes: usize,
    pub cpu_ms_per_turn: u64,
    pub request_timeout_ms: u64,
    pub bundle_hash: String,
    #[serde(default = "default_max_request_bytes")]
    pub max_request_bytes: usize,
}

pub fn default_max_request_bytes() -> usize {
    16 * 1024 * 1024
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tap {
    pub manifest: PackageManifest,
    pub bundle: Vec<u8>,
    pub source_map: Vec<u8>,
}

impl Tap {
    pub fn new(mut manifest: PackageManifest, bundle: Vec<u8>, source_map: Vec<u8>) -> Self {
        manifest.format_version = TAP_VERSION;
        manifest.bundle_hash = bundle_hash(&bundle);
        Self { manifest, bundle, source_map }
    }

    pub fn encode(&self) -> Result<Vec<u8>, PackageError> {
        let manifest = serde_json::to_vec(&self.manifest)?;
        let mut body =
            Vec::with_capacity(32 + manifest.len() + self.bundle.len() + self.source_map.len());
        body.extend_from_slice(TAP_MAGIC);
        body.extend_from_slice(&TAP_VERSION.to_le_bytes());
        body.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        body.extend_from_slice(&(self.bundle.len() as u64).to_le_bytes());
        body.extend_from_slice(&(self.source_map.len() as u64).to_le_bytes());
        body.extend_from_slice(&manifest);
        body.extend_from_slice(&self.bundle);
        body.extend_from_slice(&self.source_map);
        Ok(body)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PackageError> {
        let mut rest = bytes;
        let magic = take(&mut rest, 8)?;
        if magic != TAP_MAGIC {
            return Err(PackageError::Invalid("missing TYSELTAP magic".into()));
        }
        let version = u32::from_le_bytes(
            take(&mut rest, 4)?
                .try_into()
                .map_err(|_| PackageError::Invalid("truncated version".into()))?,
        );
        if version != TAP_VERSION {
            return Err(PackageError::Version(version));
        }
        let manifest_len = read_u64(&mut rest)? as usize;
        let bundle_len = read_u64(&mut rest)? as usize;
        let source_map_len = read_u64(&mut rest)? as usize;
        let manifest_bytes = take(&mut rest, manifest_len)?;
        let bundle = take(&mut rest, bundle_len)?.to_vec();
        let source_map = take(&mut rest, source_map_len)?.to_vec();
        if !rest.is_empty() {
            return Err(PackageError::Invalid("trailing bytes inside tap payload".into()));
        }
        let manifest: PackageManifest = serde_json::from_slice(manifest_bytes)?;
        let actual = bundle_hash(&bundle);
        if manifest.bundle_hash != actual {
            return Err(PackageError::Invalid("bundle hash mismatch".into()));
        }
        Ok(Self { manifest, bundle, source_map })
    }

    pub fn embed_into(&self, stub: &[u8]) -> Result<Vec<u8>, PackageError> {
        let payload = self.encode()?;
        let mut out = Vec::with_capacity(stub.len() + payload.len() + 16);
        out.extend_from_slice(stub);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(END_MAGIC);
        Ok(out)
    }

    pub fn extract(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() < 16 {
            return Err(PackageError::MissingPayload);
        }
        let footer = bytes.len() - 16;
        if &bytes[footer + 8..] != END_MAGIC {
            return Err(PackageError::MissingPayload);
        }
        let payload_len = {
            let mut len_bytes = [0u8; 8];
            len_bytes.copy_from_slice(&bytes[footer..footer + 8]);
            u64::from_le_bytes(len_bytes) as usize
        };
        if payload_len > footer {
            return Err(PackageError::Invalid("payload length exceeds file".into()));
        }
        let start = footer - payload_len;
        Self::decode(&bytes[start..footer])
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        Self::extract(&fs::read(path)?)
    }

    pub fn from_current_exe() -> Result<Self, PackageError> {
        Self::from_path(std::env::current_exe()?)
    }

    pub fn bundle_source(&self) -> Result<&str, PackageError> {
        std::str::from_utf8(&self.bundle)
            .map_err(|_| PackageError::Invalid("esm bundle is not utf-8".into()))
    }

    pub fn parsed_source_map(&self) -> Result<SourceMap, PackageError> {
        if self.source_map.is_empty() {
            return Err(PackageError::Invalid("source map is empty".into()));
        }
        SourceMap::parse(&self.source_map)
    }
}

pub fn bundle_hash(bundle: &[u8]) -> String {
    hex_encode(&Sha256::digest(bundle))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn read_u64(rest: &mut &[u8]) -> Result<u64, PackageError> {
    let bytes: [u8; 8] =
        take(rest, 8)?.try_into().map_err(|_| PackageError::Invalid("truncated length".into()))?;
    Ok(u64::from_le_bytes(bytes))
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], PackageError> {
    if rest.len() < n {
        return Err(PackageError::Invalid("truncated tap payload".into()));
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}
