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
        websocket: false,
        sqlite_path: String::new(),
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
fn tap_roundtrip_preserves_bundle_and_manifest() {
    let tap = sample_tap();
    let encoded = tap.encode().expect("encode");
    let decoded = Tap::decode(&encoded).expect("decode");
    assert_eq!(decoded.bundle, BUNDLE.as_bytes());
    assert_eq!(decoded.manifest.application_id, "hello-service");
    assert_eq!(decoded.manifest.bundle_hash, bundle_hash(BUNDLE.as_bytes()));
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
