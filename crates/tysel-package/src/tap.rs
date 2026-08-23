use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sourcemap::SourceMap;

pub const TAP_VERSION: u32 = 4;
pub const MIN_SUPPORTED_TAP_VERSION: u32 = 1;
pub const TAP_COMPATIBILITY_REPORT_VERSION: u32 = 1;
pub const COMPONENT_ABI_VERSION: &str = "0.4.0";
pub const MAX_TAP_PAYLOAD_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_PACKAGED_COMPONENTS: usize = 64;
pub const MAX_AOT_ARTIFACTS_PER_COMPONENT: usize = 16;

const TAP_MAGIC: &[u8; 8] = b"TYSELTAP";
const END_MAGIC: &[u8; 8] = b"TYSELEND";
const MAX_COMPONENT_INDEX_BYTES: usize = 1024 * 1024;
const MAX_COMPATIBILITY_ISSUE_BYTES: usize = 512;

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
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub websocket: bool,
    #[serde(default = "default_workers")]
    pub workers: u32,
    #[serde(default = "default_max_in_flight")]
    pub max_in_flight: u32,
    #[serde(default = "default_true")]
    pub http1: bool,
    #[serde(default)]
    pub http2: bool,
    #[serde(default)]
    pub sqlite_path: String,
    #[serde(default)]
    pub secret_names: Vec<String>,
    #[serde(default)]
    pub fetch_hosts: Vec<String>,
    #[serde(default, alias = "postgres_urls")]
    pub postgres: Vec<String>,
    #[serde(default)]
    pub fs_read: Vec<String>,
    #[serde(default)]
    pub fs_write: Vec<String>,
    #[serde(default = "default_json_logs")]
    pub json_logs: bool,
}

pub fn default_max_request_bytes() -> usize {
    16 * 1024 * 1024
}

fn default_workers() -> u32 {
    1
}

pub fn default_max_in_flight() -> u32 {
    1000
}

fn default_json_logs() -> bool {
    true
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tap {
    pub manifest: PackageManifest,
    pub bundle: Vec<u8>,
    pub source_map: Vec<u8>,
    pub components: Vec<PackagedComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedComponent {
    pub name: String,
    pub abi_version: String,
    pub source: Vec<u8>,
    pub aot: Vec<PackagedAot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedAot {
    pub target: String,
    pub wasmtime_version: String,
    pub engine_compatibility_hash: u64,
    pub source_sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

/// Machine-readable compatibility decision for one TAP payload. Reports are
/// deterministic and deliberately contain no timestamps or host-specific data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapCompatibilityReport {
    pub report_version: u32,
    pub compatible: bool,
    pub status: TapCompatibilityStatus,
    pub tap_version: Option<u32>,
    pub minimum_supported_tap_version: u32,
    pub maximum_supported_tap_version: u32,
    pub runtime_version: Option<String>,
    pub execution_profile: Option<String>,
    pub component_abi_versions: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TapCompatibilityStatus {
    Current,
    Legacy,
    UnsupportedOlder,
    UnsupportedNewer,
    Invalid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentIndex {
    components: Vec<ComponentIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentIndexEntry {
    name: String,
    abi_version: String,
    source: BlobIndex,
    aot: Vec<AotIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AotIndexEntry {
    target: String,
    wasmtime_version: String,
    engine_compatibility_hash: u64,
    source_sha256: [u8; 32],
    blob: BlobIndex,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlobIndex {
    offset: u64,
    length: u64,
    sha256: String,
}

impl Tap {
    pub fn new(mut manifest: PackageManifest, bundle: Vec<u8>, source_map: Vec<u8>) -> Self {
        manifest.format_version = TAP_VERSION;
        manifest.bundle_hash = bundle_hash(&bundle);
        Self { manifest, bundle, source_map, components: Vec::new() }
    }

    pub fn with_components(mut self, components: Vec<PackagedComponent>) -> Self {
        self.components = components;
        self
    }

    pub fn encode(&self) -> Result<Vec<u8>, PackageError> {
        validate_manifest_contract(&self.manifest, TAP_VERSION)?;
        validate_components(&self.components)?;
        let manifest = serde_json::to_vec(&self.manifest)?;
        let (component_index, component_data) = encode_components(&self.components)?;
        let expected_len = 52usize
            .checked_add(manifest.len())
            .and_then(|len| len.checked_add(self.bundle.len()))
            .and_then(|len| len.checked_add(self.source_map.len()))
            .and_then(|len| len.checked_add(component_index.len()))
            .and_then(|len| len.checked_add(component_data.len()))
            .ok_or_else(|| PackageError::Invalid("tap payload length overflow".into()))?;
        if expected_len > MAX_TAP_PAYLOAD_BYTES {
            return Err(PackageError::Invalid("tap payload exceeds size limit".into()));
        }
        let mut body = Vec::with_capacity(expected_len);
        body.extend_from_slice(TAP_MAGIC);
        body.extend_from_slice(&TAP_VERSION.to_le_bytes());
        body.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        body.extend_from_slice(&(self.bundle.len() as u64).to_le_bytes());
        body.extend_from_slice(&(self.source_map.len() as u64).to_le_bytes());
        body.extend_from_slice(&(component_index.len() as u64).to_le_bytes());
        body.extend_from_slice(&(component_data.len() as u64).to_le_bytes());
        body.extend_from_slice(&manifest);
        body.extend_from_slice(&self.bundle);
        body.extend_from_slice(&self.source_map);
        body.extend_from_slice(&component_index);
        body.extend_from_slice(&component_data);
        Ok(body)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() > MAX_TAP_PAYLOAD_BYTES {
            return Err(PackageError::Invalid("tap payload exceeds size limit".into()));
        }
        let mut rest = bytes;
        let magic = take(&mut rest, 8)?;
        if magic != TAP_MAGIC {
            return Err(PackageError::Invalid("missing TYSELTAP magic".into()));
        }
        let version = read_version(&mut rest)?;
        if !(MIN_SUPPORTED_TAP_VERSION..=TAP_VERSION).contains(&version) {
            return Err(PackageError::Version(version));
        }
        let manifest_len = read_usize(&mut rest)?;
        let bundle_len = read_usize(&mut rest)?;
        let source_map_len = read_usize(&mut rest)?;
        let (component_index_len, component_data_len) =
            if version >= 2 { (read_usize(&mut rest)?, read_usize(&mut rest)?) } else { (0, 0) };
        if component_index_len > MAX_COMPONENT_INDEX_BYTES {
            return Err(PackageError::Invalid("component index exceeds size limit".into()));
        }
        let manifest_bytes = take(&mut rest, manifest_len)?;
        let bundle = take(&mut rest, bundle_len)?.to_vec();
        let source_map = take(&mut rest, source_map_len)?.to_vec();
        let component_index = take(&mut rest, component_index_len)?;
        let component_data = take(&mut rest, component_data_len)?;
        if !rest.is_empty() {
            return Err(PackageError::Invalid("trailing bytes inside tap payload".into()));
        }
        let manifest: PackageManifest = serde_json::from_slice(manifest_bytes)?;
        validate_manifest_contract(&manifest, version)?;
        let actual = bundle_hash(&bundle);
        if manifest.bundle_hash != actual {
            return Err(PackageError::Invalid("bundle hash mismatch".into()));
        }
        let components = if version >= 2 {
            decode_components(component_index, component_data)?
        } else {
            Vec::new()
        };
        Ok(Self { manifest, bundle, source_map, components })
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
            usize::try_from(u64::from_le_bytes(len_bytes)).map_err(|_| {
                PackageError::Invalid("payload length exceeds addressable memory".into())
            })?
        };
        if payload_len > MAX_TAP_PAYLOAD_BYTES {
            return Err(PackageError::Invalid("tap payload exceeds size limit".into()));
        }
        if payload_len > footer {
            return Err(PackageError::Invalid("payload length exceeds file".into()));
        }
        let start = footer - payload_len;
        Self::decode(&bytes[start..footer])
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < 16 {
            return Err(PackageError::MissingPayload);
        }
        file.seek(SeekFrom::End(-16))?;
        let mut footer = [0u8; 16];
        file.read_exact(&mut footer)?;
        if &footer[8..] != END_MAGIC {
            return Err(PackageError::MissingPayload);
        }
        let payload_len = {
            let mut len_bytes = [0u8; 8];
            len_bytes.copy_from_slice(&footer[..8]);
            u64::from_le_bytes(len_bytes)
        };
        if payload_len > MAX_TAP_PAYLOAD_BYTES as u64 {
            return Err(PackageError::Invalid("tap payload exceeds size limit".into()));
        }
        let prefix_len = file_len - 16;
        if payload_len > prefix_len {
            return Err(PackageError::Invalid("payload length exceeds file".into()));
        }
        let payload_len = usize::try_from(payload_len).map_err(|_| {
            PackageError::Invalid("payload length exceeds addressable memory".into())
        })?;
        file.seek(SeekFrom::Start(prefix_len - payload_len as u64))?;
        let mut payload = vec![0u8; payload_len];
        file.read_exact(&mut payload)?;
        Self::decode(&payload)
    }

    pub fn from_current_exe() -> Result<Self, PackageError> {
        Self::from_path(std::env::current_exe()?)
    }

    pub fn compatibility_report(bytes: &[u8]) -> TapCompatibilityReport {
        compatibility_report(bytes)
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

pub fn compatibility_report(bytes: &[u8]) -> TapCompatibilityReport {
    let version = match tap_version(bytes) {
        Ok(version) => version,
        Err(error) => {
            return report(None, TapCompatibilityStatus::Invalid, None, vec![error.to_string()]);
        }
    };
    if version < MIN_SUPPORTED_TAP_VERSION {
        return report(
            Some(version),
            TapCompatibilityStatus::UnsupportedOlder,
            None,
            vec![format!(
                "tap version {version} predates minimum supported version {MIN_SUPPORTED_TAP_VERSION}"
            )],
        );
    }
    if version > TAP_VERSION {
        return report(
            Some(version),
            TapCompatibilityStatus::UnsupportedNewer,
            None,
            vec![format!("tap version {version} exceeds maximum supported version {TAP_VERSION}")],
        );
    }
    match Tap::decode(bytes) {
        Ok(tap) => report(
            Some(version),
            if version == TAP_VERSION {
                TapCompatibilityStatus::Current
            } else {
                TapCompatibilityStatus::Legacy
            },
            Some(&tap),
            Vec::new(),
        ),
        Err(error) => {
            report(Some(version), TapCompatibilityStatus::Invalid, None, vec![error.to_string()])
        }
    }
}

fn report(
    tap_version: Option<u32>,
    status: TapCompatibilityStatus,
    tap: Option<&Tap>,
    issues: Vec<String>,
) -> TapCompatibilityReport {
    let component_abi_versions = tap
        .into_iter()
        .flat_map(|tap| tap.components.iter().map(|component| component.abi_version.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    TapCompatibilityReport {
        report_version: TAP_COMPATIBILITY_REPORT_VERSION,
        compatible: matches!(
            status,
            TapCompatibilityStatus::Current | TapCompatibilityStatus::Legacy
        ),
        status,
        tap_version,
        minimum_supported_tap_version: MIN_SUPPORTED_TAP_VERSION,
        maximum_supported_tap_version: TAP_VERSION,
        runtime_version: tap.map(|tap| tap.manifest.runtime_version.clone()),
        execution_profile: tap.map(|tap| tap.manifest.execution_profile.clone()),
        component_abi_versions,
        issues: issues.into_iter().map(bounded_compatibility_issue).collect(),
    }
}

fn tap_version(bytes: &[u8]) -> Result<u32, PackageError> {
    let mut rest = bytes;
    let magic = take(&mut rest, 8)?;
    if magic != TAP_MAGIC {
        return Err(PackageError::Invalid("missing TYSELTAP magic".into()));
    }
    read_version(&mut rest)
}

fn read_version(rest: &mut &[u8]) -> Result<u32, PackageError> {
    Ok(u32::from_le_bytes(
        take(rest, 4)?.try_into().map_err(|_| PackageError::Invalid("truncated version".into()))?,
    ))
}

fn validate_manifest_contract(
    manifest: &PackageManifest,
    envelope_version: u32,
) -> Result<(), PackageError> {
    if manifest.format_version != envelope_version {
        return Err(PackageError::Invalid(format!(
            "manifest format version {} does not match tap envelope version {envelope_version}",
            manifest.format_version
        )));
    }
    if semver::Version::parse(&manifest.runtime_version).is_err() {
        return Err(PackageError::Invalid(
            "runtime version is not valid semantic versioning".into(),
        ));
    }
    if !matches!(manifest.execution_profile.as_str(), "service" | "isolated" | "component") {
        return Err(PackageError::Invalid("unsupported execution profile".into()));
    }
    Ok(())
}

fn bounded_compatibility_issue(mut issue: String) -> String {
    if issue.len() > MAX_COMPATIBILITY_ISSUE_BYTES {
        let mut end = MAX_COMPATIBILITY_ISSUE_BYTES;
        while !issue.is_char_boundary(end) {
            end -= 1;
        }
        issue.truncate(end);
    }
    issue
}

pub fn bundle_hash(bundle: &[u8]) -> String {
    hex_encode(&Sha256::digest(bundle))
}

fn validate_components(components: &[PackagedComponent]) -> Result<(), PackageError> {
    if components.len() > MAX_PACKAGED_COMPONENTS {
        return Err(PackageError::Invalid("too many packaged components".into()));
    }
    let mut names = BTreeSet::new();
    for component in components {
        if component.name.is_empty()
            || component.name.len() > 128
            || !component
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(PackageError::Invalid("invalid packaged component name".into()));
        }
        if !names.insert(&component.name) {
            return Err(PackageError::Invalid("duplicate packaged component name".into()));
        }
        if component.abi_version != COMPONENT_ABI_VERSION {
            return Err(PackageError::Invalid(format!(
                "unsupported component ABI version {}; expected {COMPONENT_ABI_VERSION}",
                component.abi_version
            )));
        }
        if component.aot.len() > MAX_AOT_ARTIFACTS_PER_COMPONENT {
            return Err(PackageError::Invalid(
                "too many AOT artifacts for packaged component".into(),
            ));
        }
        for aot in &component.aot {
            if aot.target.is_empty()
                || aot.target.len() > 128
                || aot.wasmtime_version.is_empty()
                || aot.wasmtime_version.len() > 64
            {
                return Err(PackageError::Invalid("invalid component AOT metadata".into()));
            }
        }
    }
    Ok(())
}

fn encode_components(components: &[PackagedComponent]) -> Result<(Vec<u8>, Vec<u8>), PackageError> {
    let mut data = Vec::new();
    let mut entries = Vec::with_capacity(components.len());
    for component in components {
        let source = append_blob(&mut data, &component.source)?;
        let mut aot = Vec::with_capacity(component.aot.len());
        for artifact in &component.aot {
            aot.push(AotIndexEntry {
                target: artifact.target.clone(),
                wasmtime_version: artifact.wasmtime_version.clone(),
                engine_compatibility_hash: artifact.engine_compatibility_hash,
                source_sha256: artifact.source_sha256,
                blob: append_blob(&mut data, &artifact.bytes)?,
            });
        }
        entries.push(ComponentIndexEntry {
            name: component.name.clone(),
            abi_version: component.abi_version.clone(),
            source,
            aot,
        });
    }
    let index = serde_json::to_vec(&ComponentIndex { components: entries })?;
    if index.len() > MAX_COMPONENT_INDEX_BYTES {
        return Err(PackageError::Invalid("component index exceeds size limit".into()));
    }
    Ok((index, data))
}

fn append_blob(data: &mut Vec<u8>, bytes: &[u8]) -> Result<BlobIndex, PackageError> {
    let offset = u64::try_from(data.len())
        .map_err(|_| PackageError::Invalid("component data offset overflow".into()))?;
    let length = u64::try_from(bytes.len())
        .map_err(|_| PackageError::Invalid("component data length overflow".into()))?;
    data.extend_from_slice(bytes);
    Ok(BlobIndex { offset, length, sha256: bundle_hash(bytes) })
}

fn decode_components(index: &[u8], data: &[u8]) -> Result<Vec<PackagedComponent>, PackageError> {
    let index: ComponentIndex = serde_json::from_slice(index)?;
    if index.components.len() > MAX_PACKAGED_COMPONENTS {
        return Err(PackageError::Invalid("too many packaged components".into()));
    }
    let mut components = Vec::with_capacity(index.components.len());
    for entry in index.components {
        if entry.aot.len() > MAX_AOT_ARTIFACTS_PER_COMPONENT {
            return Err(PackageError::Invalid(
                "too many AOT artifacts for packaged component".into(),
            ));
        }
        let source = read_blob(data, &entry.source)?;
        let mut aot = Vec::with_capacity(entry.aot.len());
        for artifact in entry.aot {
            aot.push(PackagedAot {
                target: artifact.target,
                wasmtime_version: artifact.wasmtime_version,
                engine_compatibility_hash: artifact.engine_compatibility_hash,
                source_sha256: artifact.source_sha256,
                bytes: read_blob(data, &artifact.blob)?,
            });
        }
        components.push(PackagedComponent {
            name: entry.name,
            abi_version: entry.abi_version,
            source,
            aot,
        });
    }
    validate_components(&components)?;
    Ok(components)
}

fn read_blob(data: &[u8], blob: &BlobIndex) -> Result<Vec<u8>, PackageError> {
    let offset = usize::try_from(blob.offset)
        .map_err(|_| PackageError::Invalid("component data offset overflow".into()))?;
    let length = usize::try_from(blob.length)
        .map_err(|_| PackageError::Invalid("component data length overflow".into()))?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| PackageError::Invalid("component data range overflow".into()))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| PackageError::Invalid("component data range is out of bounds".into()))?;
    if bundle_hash(bytes) != blob.sha256 {
        return Err(PackageError::Invalid("component blob hash mismatch".into()));
    }
    Ok(bytes.to_vec())
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

fn read_usize(rest: &mut &[u8]) -> Result<usize, PackageError> {
    usize::try_from(read_u64(rest)?)
        .map_err(|_| PackageError::Invalid("length exceeds addressable memory".into()))
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], PackageError> {
    if rest.len() < n {
        return Err(PackageError::Invalid("truncated tap payload".into()));
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}
