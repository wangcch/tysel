//! TypeScript type-check, transpile, bundle, and native executable assembly.
//!
//! Spike C copies a prebuilt runtime stub and appends a TAP trailer containing
//! the ESM bundle, embedded manifest, and source map.

mod bundle;
mod evidence;
mod reproducibility;
mod signing;
mod supply_chain;
mod transpile;

use std::fs;
use std::path::Path;

use tysel_engine_wasm::{COMPONENT_ABI_VERSION, ComponentEngineConfig, WasmComponentEngine};
use tysel_manifest::Manifest;
use tysel_package::{PackageManifest, PackagedAot, PackagedComponent, Tap, identity_source_map};

pub use evidence::{
    RELEASE_EVIDENCE_VERSION, ReleaseArtifactEvidence, ReleaseDocumentEvidence,
    ReleaseEvidenceIndex, ReleaseSidecars, ReleaseSupplyChainEvidence, verify_release_evidence,
    write_release_evidence,
};
pub use reproducibility::{
    REPRODUCIBLE_BUILD_EVIDENCE_VERSION, ReproducibleArtifact, ReproducibleBuild,
    ReproducibleBuildEvidence, compare_reproducible_builds, verify_reproducible_build_evidence,
    write_reproducible_build_evidence,
};
pub use signing::{
    RELEASE_ARTIFACT_SIGNATURE_VERSION, RELEASE_SIGNATURE_VERSION, ReleaseArtifactSignature,
    ReleaseKeyInfo, ReleaseKeyStatus, ReleaseSignature, TRUST_POLICY_VERSION, TrustPolicy,
    TrustedReleaseKey, release_key_info, sign_release_artifact, sign_release_evidence,
    sign_release_metadata, validate_trust_policy, validate_trust_policy_transition,
    verify_release_artifact_signature, verify_release_metadata_signature, verify_release_signature,
};
pub use supply_chain::{
    CycloneDxBom, LicenseInventory, RuntimeInventory, SUPPLY_CHAIN_VERSION, SupplyChainComponent,
    embedded_runtime_inventory,
};
pub use transpile::transpile_typescript;

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

pub fn tap_from_app(
    manifest: &Manifest,
    runtime_version: &str,
    bundle: Vec<u8>,
    source_map: Vec<u8>,
) -> Tap {
    Tap::new(package_manifest(manifest, runtime_version), bundle, source_map)
}

/// Validate, AOT-compile, and package a portable Component. The source is
/// always retained so another target can safely reject the AOT and recompile.
pub fn tap_from_component(
    manifest: &Manifest,
    runtime_version: &str,
    source: Vec<u8>,
) -> anyhow::Result<Tap> {
    let engine = WasmComponentEngine::new(ComponentEngineConfig::default())?;
    admit_component_imports(manifest, &engine.compile(&source)?)?;
    let artifact = engine.precompile(&source)?;
    let component = PackagedComponent {
        name: manifest.app.name.clone(),
        abi_version: artifact.component_abi_version,
        source,
        aot: vec![PackagedAot {
            target: artifact.target,
            wasmtime_version: artifact.wasmtime_version,
            engine_compatibility_hash: artifact.engine_compatibility_hash,
            source_sha256: artifact.source_sha256,
            bytes: artifact.bytes,
        }],
    };
    Ok(Tap::new(package_manifest(manifest, runtime_version), Vec::new(), Vec::new())
        .with_components(vec![component]))
}

/// Validate only the stable Component task ABI. Kept for source compatibility
/// with callers that do not have a project manifest.
pub fn validate_component(source: &[u8]) -> anyhow::Result<()> {
    let engine = WasmComponentEngine::new(ComponentEngineConfig::default())?;
    engine.compile(source)?;
    Ok(())
}

/// Validate the Component ABI and admit every import against the manifest.
pub fn validate_component_for_manifest(manifest: &Manifest, source: &[u8]) -> anyhow::Result<()> {
    let engine = WasmComponentEngine::new(ComponentEngineConfig::default())?;
    let component = engine.compile(source)?;
    admit_component_imports(manifest, &component)?;
    Ok(())
}

/// Package portable source for local one-shot execution without generating an
/// AOT blob that the unsigned development path cannot load.
pub fn tap_from_component_portable(
    manifest: &Manifest,
    runtime_version: &str,
    source: Vec<u8>,
) -> anyhow::Result<Tap> {
    let engine = WasmComponentEngine::new(ComponentEngineConfig::default())?;
    let component = engine.compile(&source)?;
    admit_component_imports(manifest, &component)?;
    Ok(Tap::new(package_manifest(manifest, runtime_version), Vec::new(), Vec::new())
        .with_components(vec![PackagedComponent {
            name: manifest.app.name.clone(),
            abi_version: COMPONENT_ABI_VERSION.into(),
            source,
            aot: Vec::new(),
        }]))
}

fn admit_component_imports(
    manifest: &Manifest,
    component: &tysel_engine_wasm::CompiledComponent,
) -> anyhow::Result<()> {
    for import in component.required_imports() {
        let declared = match (
            import.id.0.as_str(),
            import.interface.as_str(),
            import.version.major,
            import.version.minor,
            import.version.patch,
        ) {
            ("tysel:fs", "read", 0, 4, 0) => !manifest.permissions.fs_read.is_empty(),
            ("tysel:fs", "write", 0, 4, 0) => !manifest.permissions.fs_write.is_empty(),
            _ => anyhow::bail!("unsupported Component capability import {import}"),
        };
        if !declared {
            anyhow::bail!("Component capability import {import} is not declared in the manifest");
        }
    }
    Ok(())
}

fn package_manifest(manifest: &Manifest, runtime_version: &str) -> PackageManifest {
    PackageManifest {
        format_version: 0,
        runtime_version: runtime_version.to_owned(),
        application_id: manifest.app.name.clone(),
        entrypoint: manifest.app.entry.clone(),
        execution_profile: manifest.app.profile.clone(),
        listen: manifest.server.listen.clone(),
        memory_limit_bytes: (manifest.limits.memory_mb as usize).saturating_mul(1024 * 1024),
        cpu_ms_per_turn: manifest.limits.cpu_ms_per_turn,
        request_timeout_ms: manifest.limits.request_timeout_ms,
        bundle_hash: String::new(),
        max_request_bytes: (manifest.limits.max_request_mb as usize).saturating_mul(1024 * 1024),
        max_response_bytes: (manifest.limits.max_response_mb as usize).saturating_mul(1024 * 1024),
        websocket: manifest.server.websocket,
        workers: manifest.server.workers,
        max_in_flight: manifest.limits.max_in_flight,
        http1: manifest.server.http1,
        http2: manifest.server.http2,
        sqlite_path: if manifest.durable.store == "sqlite" {
            manifest.durable.path.clone()
        } else {
            String::new()
        },
        secret_names: manifest.permissions.secrets.clone(),
        fetch_hosts: manifest.permissions.fetch.clone(),
        postgres: manifest.permissions.postgres.clone(),
        fs_read: manifest.permissions.fs_read.clone(),
        fs_write: manifest.permissions.fs_write.clone(),
        json_logs: manifest.observability.logs.eq_ignore_ascii_case("json"),
    }
}

pub fn embed(stub: impl AsRef<Path>, output: impl AsRef<Path>, tap: &Tap) -> anyhow::Result<()> {
    let stub = stub.as_ref();
    let output = output.as_ref();
    let stub_bytes = fs::read(stub)?;
    let packaged = tap.embed_into(&stub_bytes)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, packaged)?;
    set_executable(output)?;
    Ok(())
}

pub fn read_bundle(entry: impl AsRef<Path>) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let entry = entry.as_ref();
    let source = fs::read(entry)?;
    let display = entry.to_string_lossy();
    let ext = entry.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match ext {
        "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" => {
            let text = std::str::from_utf8(&source)
                .map_err(|_| anyhow::anyhow!("source bundle must be utf-8"))?;
            if bundle::has_runtime_imports(entry, text)? {
                bundle::bundle(entry)
            } else {
                match ext {
                    "ts" | "mts" | "cts" => transpile::transpile_typescript(entry, text),
                    _ => {
                        let map = identity_source_map(&display, text)?;
                        Ok((source, map))
                    }
                }
            }
        }
        other => anyhow::bail!("unsupported entry extension '.{other}'"),
    }
}

/// Parse runtime module specifiers without matching comments or string contents.
pub fn module_specifiers(path: impl AsRef<Path>, source: &str) -> anyhow::Result<Vec<String>> {
    bundle::module_specifiers(path.as_ref(), source)
}

fn set_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_package::SourceMap;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn embed_appends_tap_trailer_to_stub() {
        let dir = std::env::temp_dir().join(format!("tysel-build-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("stub");
        let output = dir.join("app");
        fs::write(&stub, b"stub-runtime").unwrap();
        let manifest = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
        )
        .unwrap();
        let bundle = b"export default { fetch() { return new Response(\"ok\"); } };".to_vec();
        let map = identity_source_map("src/index.ts", "export default {}\n").unwrap();
        let tap = tap_from_app(&manifest, "0.0.1", bundle.clone(), map);
        embed(&stub, &output, &tap).unwrap();
        let extracted = Tap::from_path(&output).unwrap();
        assert!(fs::read(&output).unwrap().starts_with(b"stub-runtime"));
        assert_eq!(extracted.bundle, bundle);
        assert_eq!(extracted.manifest.application_id, "hello-service");
        assert_eq!(extracted.manifest.listen, "127.0.0.1:0");
        assert_eq!(extracted.manifest.sqlite_path, "./data/tysel.db");
        assert!(extracted.manifest.secret_names.is_empty());
        assert!(extracted.manifest.fetch_hosts.is_empty());
        assert!(extracted.manifest.json_logs);
    }

    #[test]
    fn packages_portable_component_with_validated_host_aot() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "echo-component"
entry = "echo.wasm"
profile = "component"
"#,
        )
        .unwrap();
        let source = wat::parse_str(
            r#"
(component
  (core module $module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc")
      (param i32 i32 i32) (param $new-len i32) (result i32)
      (local $ptr i32)
      global.get $heap
      local.tee $ptr
      local.get $new-len
      i32.add
      global.set $heap
      local.get $ptr)
    (func (export "run") (param $ptr i32) (param $len i32) (result i32)
      i32.const 0
      i32.const 0
      i32.store
      i32.const 4
      local.get $ptr
      i32.store
      i32.const 8
      local.get $len
      i32.store
      i32.const 0))
  (core instance $instance (instantiate $module))
  (alias core export $instance "memory" (core memory $memory))
  (alias core export $instance "realloc" (core func $realloc))
  (alias core export $instance "run" (core func $run-core))
  (type $run-type
    (func (param "input" string) (result (result string (error string)))))
  (func $run (type $run-type)
    (canon lift (core func $run-core) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#,
        )
        .unwrap();
        let tap = tap_from_component(&manifest, "0.4.0", source.clone()).unwrap();
        assert!(tap.bundle.is_empty());
        assert_eq!(tap.components.len(), 1);
        assert_eq!(tap.components[0].source, source);
        assert_eq!(tap.components[0].abi_version, "0.4.0");
        assert_eq!(tap.components[0].aot.len(), 1);
        validate_component(&source).unwrap();
        let portable = tap_from_component_portable(&manifest, "0.4.0", source).unwrap();
        assert!(portable.components[0].aot.is_empty());
    }

    #[test]
    fn tap_disables_json_logs_when_configured_off() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[observability]
logs = "off"
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert!(!tap.manifest.json_logs);
    }

    #[test]
    fn tap_copies_fetch_hosts() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[permissions]
fetch = ["api.openai.com"]
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert_eq!(tap.manifest.fetch_hosts, ["api.openai.com"]);
    }

    #[test]
    fn tap_copies_http_protocols() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "http2-service"
entry = "src/index.ts"

[server]
http1 = false
http2 = true
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert!(!tap.manifest.http1);
        assert!(tap.manifest.http2);
    }

    #[test]
    fn tap_copies_server_workers() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "parallel-service"
entry = "src/index.ts"
profile = "service"

[server]
workers = 2
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert_eq!(tap.manifest.workers, 2);
    }

    #[test]
    fn tap_copies_max_in_flight() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "bounded-service"
entry = "src/index.ts"

[limits]
max_in_flight = 17
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert_eq!(tap.manifest.max_in_flight, 17);
    }

    #[test]
    fn tap_copies_max_response_bytes() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "bounded-service"
entry = "src/index.ts"

[limits]
max_response_mb = 7
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert_eq!(tap.manifest.max_response_bytes, 7 * 1024 * 1024);
    }

    #[test]
    fn tap_copies_postgres_and_fs_permissions() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[permissions]
postgres = ["main:read-write"]
fs_read = ["./data"]
fs_write = ["./data"]
"#,
        )
        .unwrap();
        let tap = tap_from_app(
            &manifest,
            "0.0.1",
            b"export default {};".to_vec(),
            identity_source_map("src/index.ts", "export default {}\n").unwrap(),
        );
        assert_eq!(tap.manifest.postgres, ["main:read-write"]);
        assert_eq!(tap.manifest.fs_read, ["./data"]);
        assert_eq!(tap.manifest.fs_write, ["./data"]);
    }

    #[test]
    fn transpiles_typescript_entry_and_maps_back_to_source() {
        let dir = std::env::temp_dir().join(format!("tysel-build-ts-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(
            &entry,
            r#"
export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({ message: "Hello from Tysel" });
  },
};
"#,
        )
        .unwrap();
        let (bundle, map) = read_bundle(&entry).unwrap();
        let js = String::from_utf8(bundle).unwrap();
        assert!(js.contains("export default"));
        assert!(!js.contains("Promise<Response>"));
        let parsed = tysel_package::SourceMap::parse(&map).unwrap();
        let origin = parsed.original_position(1, 1).unwrap();
        assert!(origin.source.ends_with("src/index.ts"));
        assert!(origin.content.unwrap().contains("request: Request"));
    }

    #[test]
    fn bundles_relative_typescript_imports() {
        let dir = std::env::temp_dir().join(format!("tysel-build-rel-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/util.ts"), "export const message: string = \"bundled\";\n")
            .unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(
            &entry,
            r#"
import { message } from "./util.ts";
export default {
  fetch() {
    return Response.json({ message });
  },
};
"#,
        )
        .unwrap();
        let (bundle, _) = read_bundle(&entry).unwrap();
        let js = String::from_utf8(bundle).unwrap();
        assert!(js.contains("__tysel_require"));
        assert!(js.contains("bundled"));
        assert!(!js.contains("from \"./util.ts\""));
        assert!(js.contains("export default"));
    }

    #[test]
    fn bundled_source_map_locates_imported_typescript() {
        let dir = std::env::temp_dir().join(format!("tysel-build-smap-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/helper.ts"),
            r#"
export function marker(): string {
  return "SOURCEMAP_MARKER";
}
"#,
        )
        .unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(
            &entry,
            r#"
import { marker } from "./helper.ts";
export default {
  fetch() {
    return new Response(marker());
  },
};
"#,
        )
        .unwrap();
        let (bundle, map_bytes) = read_bundle(&entry).unwrap();
        let js = String::from_utf8(bundle).unwrap();
        let map_json = String::from_utf8(map_bytes.clone()).unwrap();
        assert!(!map_json.contains("\"mappings\": \"AAAA\""), "bundled map was a stub: {map_json}");
        let map = SourceMap::parse(&map_bytes).expect("parse bundled map");
        let line = line_of(&js, "SOURCEMAP_MARKER");
        let origin = map.original_position(line, 3).expect("locate marker");
        assert!(origin.source.ends_with("src/helper.ts"), "source was {}", origin.source);
        assert!(
            origin.content.as_deref().unwrap().contains("SOURCEMAP_MARKER"),
            "missing original helper source"
        );
        assert!(origin.content.as_deref().unwrap().contains("marker(): string"));
    }

    fn line_of(source: &str, needle: &str) -> u32 {
        let offset = source.find(needle).unwrap_or_else(|| panic!("missing {needle}"));
        source[..offset].bytes().filter(|byte| *byte == b'\n').count() as u32 + 1
    }

    #[test]
    fn bundles_bare_specifier_from_node_modules() {
        let dir = std::env::temp_dir().join(format!("tysel-build-hono-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules/hono")).unwrap();
        fs::write(
            dir.join("node_modules/hono/package.json"),
            r#"{"name":"hono","type":"module","exports":{".":"./index.js"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("node_modules/hono/index.js"),
            r#"
export class Hono {
  constructor() {
    this.routes = [];
  }
  get(path, handler) {
    this.routes.push(["GET", path, handler]);
    return this;
  }
  fetch(request) {
    const url = new URL(request.url);
    for (const [method, path, handler] of this.routes) {
      if (method !== request.method) continue;
      const params = matchRoute(path, url.pathname);
      if (!params) continue;
      return handler({
        json: (data, status) => Response.json(data, { status: status || 200 }),
        req: { param: (key) => params[key] },
      });
    }
    return new Response("not found", { status: 404 });
  }
}
function matchRoute(pattern, pathname) {
  if (pattern === pathname) return {};
  const patternParts = pattern.split("/").filter(Boolean);
  const pathParts = pathname.split("/").filter(Boolean);
  if (patternParts.length !== pathParts.length) return null;
  const params = {};
  for (let i = 0; i < patternParts.length; i++) {
    if (patternParts[i].startsWith(":")) params[patternParts[i].slice(1)] = pathParts[i];
    else if (patternParts[i] !== pathParts[i]) return null;
  }
  return params;
}
"#,
        )
        .unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(
            &entry,
            r#"
import { Hono } from "hono";
const app = new Hono();
app.get("/", (c) => c.json({ ok: true }));
app.get("/hello/:name", (c) => c.json({ hello: c.req.param("name") }));
export default app;
"#,
        )
        .unwrap();
        let (bundle, _) = read_bundle(&entry).unwrap();
        let js = String::from_utf8(bundle).unwrap();
        assert!(js.contains("class Hono"));
        assert!(js.contains("__tysel_require"));
        assert!(!js.contains("from \"hono\""));
    }

    #[test]
    fn json_modules_are_parsed_not_evaled_as_object_literals() {
        let dir = std::env::temp_dir().join(format!("tysel-build-json-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/data.json"), r#"{"__proto__":{"polluted":true},"ok":1}"#).unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(
            &entry,
            r#"
import data from "./data.json";
export default {
  fetch() {
    return Response.json({ ok: data.ok });
  },
};
"#,
        )
        .unwrap();
        let (bundle, _) = read_bundle(&entry).unwrap();
        let js = String::from_utf8(bundle).unwrap();
        assert!(js.contains("JSON.parse("));
        assert!(!js.contains("module.exports = {"));
    }

    #[test]
    fn node_builtins_are_rejected() {
        let dir = std::env::temp_dir().join(format!("tysel-build-fs-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        let entry = dir.join("src/index.ts");
        fs::write(&entry, "import fs from \"fs\";\nexport default { fetch() { return fs; } };\n")
            .unwrap();
        let err = read_bundle(&entry).unwrap_err().to_string();
        assert!(err.contains("builtin") || err.contains("node:fs"), "error was {err}");
    }
}
