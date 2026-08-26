use super::*;
use crate::tap::{PackageError, bundle_hash};

fn sample_manifest() -> PackageManifest {
    PackageManifest {
        format_version: 0,
        runtime_version: "0.0.1".into(),
        application_id: "hello-service".into(),
        entrypoint: "src/index.ts".into(),
        execution_profile: "service".into(),
        listen: "127.0.0.1:0".into(),
        memory_limit_bytes: 8 * 1024 * 1024,
        cpu_ms_per_turn: 50,
        request_timeout_ms: 2_000,
        bundle_hash: String::new(),
        max_request_bytes: 16 * 1024 * 1024,
        max_response_bytes: 16 * 1024 * 1024,
        websocket: false,
        workers: 1,
        max_in_flight: 1000,
        http1: true,
        http2: false,
        sqlite_path: String::new(),
        secret_names: Vec::new(),
        fetch_hosts: Vec::new(),
        postgres: Vec::new(),
        fs_read: Vec::new(),
        fs_write: Vec::new(),
        json_logs: true,
    }
}

const BUNDLE: &str = "export default { async fetch() { return new Response(\"ok\"); } };\n";
const TYPESCRIPT: &str = "export default {\n  async fetch(request: Request): Promise<Response> {\n    return new Response(\"ok\");\n  },\n};\n";

fn sample_tap() -> Tap {
    let map = identity_source_map("src/index.ts", TYPESCRIPT).expect("source map");
    Tap::new(sample_manifest(), BUNDLE.as_bytes().to_vec(), map)
}

#[test]
fn crate_is_named() {
    assert!(!crate_name().is_empty());
}

#[test]
fn runtime_manifest_matches_tap_and_component_contracts() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../runtime-js/compatibility.json"))
            .expect("runtime compatibility manifest");

    assert_eq!(manifest["tap"]["minimumSupportedVersion"], MIN_SUPPORTED_TAP_VERSION);
    assert_eq!(manifest["tap"]["maximumSupportedVersion"], TAP_VERSION);
    assert_eq!(manifest["componentAbiVersion"], COMPONENT_ABI_VERSION);
}

#[test]
fn tap_roundtrip_preserves_bundle_and_manifest() {
    let tap = sample_tap();
    let encoded = tap.encode().expect("encode");
    let decoded = Tap::decode(&encoded).expect("decode");
    assert_eq!(decoded.bundle, BUNDLE.as_bytes());
    assert_eq!(decoded.manifest.application_id, "hello-service");
    assert_eq!(decoded.manifest.bundle_hash, bundle_hash(BUNDLE.as_bytes()));
    assert!(decoded.manifest.json_logs);
}

#[test]
fn trailer_survives_embedding_in_a_stub() {
    let tap = sample_tap();
    let packaged = tap.embed_into(b"\x7fstub-bytes").expect("embed");
    assert!(packaged.starts_with(b"\x7fstub-bytes"));
    let extracted = Tap::extract(&packaged).expect("extract");
    assert_eq!(extracted.bundle, tap.bundle);
    assert_eq!(extracted.manifest.listen, "127.0.0.1:0");
}

#[test]
fn missing_trailer_is_a_missing_payload() {
    let err = Tap::extract(b"just-a-runtime-stub").expect_err("no trailer");
    assert!(matches!(err, PackageError::MissingPayload));
}

#[test]
fn from_path_reads_only_the_trailer() {
    let tap = sample_tap();
    let stub = vec![0x7f; 2 * 1024 * 1024];
    let packaged = tap.embed_into(&stub).expect("embed");
    let path = std::env::temp_dir().join(format!(
        "tysel-tap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::write(&path, &packaged).expect("write packaged stub");
    let extracted = Tap::from_path(&path).expect("from_path");
    let _ = std::fs::remove_file(&path);
    assert_eq!(extracted.bundle, tap.bundle);
    assert_eq!(extracted.manifest.application_id, "hello-service");
}

#[test]
fn source_map_locates_typescript_origin() {
    let tap = sample_tap();
    let map = tap.parsed_source_map().expect("parse map");
    let first = map.original_position(1, 1).expect("line 1");
    assert_eq!(first.source, "src/index.ts");
    assert_eq!(first.line, 1);
    assert_eq!(first.column, 1);
    let second = map.original_position(2, 1).expect("line 2");
    assert_eq!(second.line, 2);
    assert!(second.content.as_deref().unwrap().contains("request: Request"));
}

#[test]
fn source_map_writer_roundtrips_line_mappings() {
    let mut writer = SourceMapWriter::new();
    writer.add(0, 0, 0, 0);
    writer.end_line();
    writer.add(0, 0, 1, 0);
    let json = writer
        .into_json("app.js", vec!["src/index.ts".into()], vec!["a\nb\n".into()])
        .expect("json");
    let map = SourceMap::parse(&json).expect("parse");
    let first = map.original_position(1, 1).expect("line 1");
    assert_eq!(first.source, "src/index.ts");
    assert_eq!(first.line, 1);
    let second = map.original_position(2, 1).expect("line 2");
    assert_eq!(second.line, 2);
}

#[test]
fn bundle_hash_mismatch_is_rejected() {
    let mut tap = sample_tap();
    tap.manifest.bundle_hash = "deadbeef".into();
    let encoded = tap.encode().expect("encode");
    let err = Tap::decode(&encoded).expect_err("hash");
    assert!(matches!(err, PackageError::Invalid(message) if message.contains("hash")));
}

#[test]
fn current_tap_roundtrips_portable_and_aot_components() {
    let component = PackagedComponent {
        name: "echo".into(),
        abi_version: "0.4.0".into(),
        source: b"portable-component".to_vec(),
        aot: vec![PackagedAot {
            target: "aarch64-macos".into(),
            wasmtime_version: "32.0.1".into(),
            engine_compatibility_hash: 42,
            source_sha256: [7; 32],
            bytes: b"native-aot".to_vec(),
        }],
    };
    let tap = sample_tap().with_components(vec![component.clone()]);
    let decoded = Tap::decode(&tap.encode().unwrap()).unwrap();
    assert_eq!(decoded.manifest.format_version, TAP_VERSION);
    assert_eq!(decoded.components, [component]);
}

#[test]
fn tap_v2_without_protocol_fields_remains_readable() {
    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest["format_version"] = 2.into();
    manifest["bundle_hash"] = bundle_hash(BUNDLE.as_bytes()).into();
    let object = manifest.as_object_mut().unwrap();
    object.remove("http1");
    object.remove("http2");
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let map = identity_source_map("src/index.ts", TYPESCRIPT).unwrap();
    let component_index = br#"{"components":[]}"#;
    let mut encoded = Vec::new();
    encoded.extend_from_slice(b"TYSELTAP");
    encoded.extend_from_slice(&2u32.to_le_bytes());
    encoded.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(BUNDLE.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(map.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(component_index.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&0u64.to_le_bytes());
    encoded.extend_from_slice(&manifest);
    encoded.extend_from_slice(BUNDLE.as_bytes());
    encoded.extend_from_slice(&map);
    encoded.extend_from_slice(component_index);

    let decoded = Tap::decode(&encoded).unwrap();
    assert_eq!(decoded.manifest.format_version, 2);
    assert!(decoded.manifest.http1);
    assert!(!decoded.manifest.http2);
    assert_eq!(compatibility_report(&encoded).status, TapCompatibilityStatus::Legacy);
}

#[test]
fn tap_v3_without_worker_or_admission_fields_remains_readable() {
    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest["format_version"] = 3.into();
    manifest["bundle_hash"] = bundle_hash(BUNDLE.as_bytes()).into();
    let object = manifest.as_object_mut().unwrap();
    object.remove("workers");
    object.remove("max_in_flight");
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let map = identity_source_map("src/index.ts", TYPESCRIPT).unwrap();
    let component_index = br#"{"components":[]}"#;
    let mut encoded = Vec::new();
    encoded.extend_from_slice(b"TYSELTAP");
    encoded.extend_from_slice(&3u32.to_le_bytes());
    encoded.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(BUNDLE.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(map.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(component_index.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&0u64.to_le_bytes());
    encoded.extend_from_slice(&manifest);
    encoded.extend_from_slice(BUNDLE.as_bytes());
    encoded.extend_from_slice(&map);
    encoded.extend_from_slice(component_index);

    let decoded = Tap::decode(&encoded).unwrap();
    assert_eq!(decoded.manifest.format_version, 3);
    assert_eq!(decoded.manifest.workers, 1);
    assert_eq!(decoded.manifest.max_in_flight, 1000);
    assert_eq!(compatibility_report(&encoded).status, TapCompatibilityStatus::Legacy);
}

#[test]
fn manifests_without_workers_default_to_one() {
    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest.as_object_mut().unwrap().remove("workers");

    let decoded: PackageManifest = serde_json::from_value(manifest).unwrap();
    assert_eq!(decoded.workers, 1);
}

#[test]
fn manifests_without_max_in_flight_use_the_historical_default() {
    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest.as_object_mut().unwrap().remove("max_in_flight");

    let decoded: PackageManifest = serde_json::from_value(manifest).unwrap();
    assert_eq!(decoded.max_in_flight, 1000);
}

#[test]
fn manifests_without_max_response_bytes_use_the_historical_default() {
    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest.as_object_mut().unwrap().remove("max_response_bytes");

    let decoded: PackageManifest = serde_json::from_value(manifest).unwrap();
    assert_eq!(decoded.max_response_bytes, 16 * 1024 * 1024);
}

#[test]
fn component_blob_tampering_is_rejected() {
    let component = PackagedComponent {
        name: "echo".into(),
        abi_version: "0.4.0".into(),
        source: b"portable-component".to_vec(),
        aot: Vec::new(),
    };
    let mut encoded = sample_tap().with_components(vec![component]).encode().unwrap();
    *encoded.last_mut().unwrap() ^= 1;
    let error = Tap::decode(&encoded).unwrap_err();
    assert!(matches!(error, PackageError::Invalid(message) if message.contains("hash")));
}

#[test]
fn unsupported_component_abi_is_rejected_and_reported_incompatible() {
    let component = PackagedComponent {
        name: "echo".into(),
        abi_version: "0.5.0".into(),
        source: b"portable-component".to_vec(),
        aot: Vec::new(),
    };
    let error = sample_tap().with_components(vec![component]).encode().unwrap_err();
    assert!(
        matches!(error, PackageError::Invalid(message) if message.contains("unsupported component ABI version"))
    );

    let supported = PackagedComponent {
        name: "echo".into(),
        abi_version: COMPONENT_ABI_VERSION.into(),
        source: b"portable-component".to_vec(),
        aot: Vec::new(),
    };
    let mut encoded = sample_tap().with_components(vec![supported]).encode().unwrap();
    let offset = encoded
        .windows(COMPONENT_ABI_VERSION.len())
        .position(|window| window == COMPONENT_ABI_VERSION.as_bytes())
        .expect("component ABI in index");
    encoded[offset..offset + COMPONENT_ABI_VERSION.len()].copy_from_slice(b"0.5.0");
    let report = compatibility_report(&encoded);
    assert!(!report.compatible);
    assert_eq!(report.status, TapCompatibilityStatus::Invalid);
    assert!(report.issues.iter().any(|issue| issue.contains("unsupported component ABI version")));
}

#[test]
fn tap_v1_payloads_remain_readable() {
    let mut manifest = sample_manifest();
    manifest.format_version = 1;
    manifest.bundle_hash = bundle_hash(BUNDLE.as_bytes());
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let map = identity_source_map("src/index.ts", TYPESCRIPT).unwrap();
    let mut encoded = Vec::new();
    encoded.extend_from_slice(b"TYSELTAP");
    encoded.extend_from_slice(&1u32.to_le_bytes());
    encoded.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(BUNDLE.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&(map.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&manifest);
    encoded.extend_from_slice(BUNDLE.as_bytes());
    encoded.extend_from_slice(&map);

    let decoded = Tap::decode(&encoded).unwrap();
    assert_eq!(decoded.manifest.format_version, 1);
    assert!(decoded.components.is_empty());
}

#[test]
fn compatibility_report_is_deterministic_for_current_and_legacy_taps() {
    let current = sample_tap().encode().unwrap();
    let current_report = compatibility_report(&current);
    assert!(current_report.compatible);
    assert_eq!(current_report.status, TapCompatibilityStatus::Current);
    assert_eq!(current_report.tap_version, Some(TAP_VERSION));
    assert_eq!(current_report.runtime_version.as_deref(), Some("0.0.1"));
    assert_eq!(current_report.execution_profile.as_deref(), Some("service"));
    assert!(current_report.issues.is_empty());
    assert_eq!(
        serde_json::to_string(&current_report).unwrap(),
        serde_json::to_string(&compatibility_report(&current)).unwrap()
    );

    let mut manifest = sample_manifest();
    manifest.format_version = 1;
    manifest.bundle_hash = bundle_hash(BUNDLE.as_bytes());
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let map = identity_source_map("src/index.ts", TYPESCRIPT).unwrap();
    let mut legacy = Vec::new();
    legacy.extend_from_slice(b"TYSELTAP");
    legacy.extend_from_slice(&1u32.to_le_bytes());
    legacy.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    legacy.extend_from_slice(&(BUNDLE.len() as u64).to_le_bytes());
    legacy.extend_from_slice(&(map.len() as u64).to_le_bytes());
    legacy.extend_from_slice(&manifest);
    legacy.extend_from_slice(BUNDLE.as_bytes());
    legacy.extend_from_slice(&map);

    let legacy_report = Tap::compatibility_report(&legacy);
    assert!(legacy_report.compatible);
    assert_eq!(legacy_report.status, TapCompatibilityStatus::Legacy);
    assert_eq!(legacy_report.tap_version, Some(1));
}

#[test]
fn compatibility_report_distinguishes_future_old_and_invalid_payloads() {
    let envelope = |version: u32| {
        let mut bytes = b"TYSELTAP".to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes
    };
    let future = compatibility_report(&envelope(TAP_VERSION + 1));
    assert!(!future.compatible);
    assert_eq!(future.status, TapCompatibilityStatus::UnsupportedNewer);
    assert_eq!(future.tap_version, Some(TAP_VERSION + 1));

    let old = compatibility_report(&envelope(MIN_SUPPORTED_TAP_VERSION - 1));
    assert!(!old.compatible);
    assert_eq!(old.status, TapCompatibilityStatus::UnsupportedOlder);

    let invalid = compatibility_report(b"not-a-tap");
    assert!(!invalid.compatible);
    assert_eq!(invalid.status, TapCompatibilityStatus::Invalid);
    assert_eq!(invalid.tap_version, None);
}

#[test]
fn stable_tap_contract_rejects_ambiguous_or_unknown_metadata() {
    let mut mismatch = sample_tap();
    mismatch.manifest.format_version = TAP_VERSION - 1;
    let error = mismatch.encode().unwrap_err();
    assert!(matches!(error, PackageError::Invalid(message) if message.contains("does not match")));

    let mut manifest = sample_manifest();
    manifest.format_version = TAP_VERSION;
    manifest.bundle_hash = bundle_hash(BUNDLE.as_bytes());
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let mut ambiguous = Vec::new();
    ambiguous.extend_from_slice(b"TYSELTAP");
    ambiguous.extend_from_slice(&(TAP_VERSION - 1).to_le_bytes());
    ambiguous.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    ambiguous.extend_from_slice(&(BUNDLE.len() as u64).to_le_bytes());
    ambiguous.extend_from_slice(&0u64.to_le_bytes());
    ambiguous.extend_from_slice(&0u64.to_le_bytes());
    ambiguous.extend_from_slice(&0u64.to_le_bytes());
    ambiguous.extend_from_slice(&manifest);
    ambiguous.extend_from_slice(BUNDLE.as_bytes());
    let error = Tap::decode(&ambiguous).unwrap_err();
    assert!(matches!(error, PackageError::Invalid(message) if message.contains("does not match")));
    assert_eq!(compatibility_report(&ambiguous).status, TapCompatibilityStatus::Invalid);

    let mut unknown_profile = sample_tap();
    unknown_profile.manifest.execution_profile = "future-profile".into();
    let error = unknown_profile.encode().unwrap_err();
    assert!(
        matches!(error, PackageError::Invalid(message) if message.contains("execution profile"))
    );

    let mut invalid_runtime = sample_tap();
    invalid_runtime.manifest.runtime_version = "development".into();
    let error = invalid_runtime.encode().unwrap_err();
    assert!(
        matches!(error, PackageError::Invalid(message) if message.contains("semantic versioning"))
    );

    let mut manifest = serde_json::to_value(sample_manifest()).unwrap();
    manifest["unknownCompatibilityFlag"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PackageManifest>(manifest).is_err());

    let mut report =
        serde_json::to_value(compatibility_report(&sample_tap().encode().unwrap())).unwrap();
    report["futureSecurityFlag"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TapCompatibilityReport>(report).is_err());
}

#[test]
fn source_map_symbolicates_generated_stack_frames() {
    let map =
        SourceMap::parse(&identity_source_map("src/index.ts", "first\nsecond\n").unwrap()).unwrap();
    let stack = "Error: failure\n    at fetch (app.js:2:1)";
    let mapped = map.symbolicate_stack(stack);
    assert!(mapped.contains("src/index.ts:2:1"), "{mapped}");
    assert!(!mapped.contains("app.js:"), "{mapped}");
}
