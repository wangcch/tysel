//! TypeScript type-check, transpile, bundle, and native executable assembly.
//!
//! Spike C copies a prebuilt runtime stub and appends a TAP trailer containing
//! the ESM bundle, embedded manifest, and source map.

mod bundle;
mod transpile;

use std::fs;
use std::path::Path;

use tysel_manifest::Manifest;
use tysel_package::{PackageManifest, Tap, identity_source_map};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

pub fn tap_from_app(
    manifest: &Manifest,
    runtime_version: &str,
    bundle: Vec<u8>,
    source_map: Vec<u8>,
) -> Tap {
    let packaged = PackageManifest {
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
    };
    Tap::new(packaged, bundle, source_map)
}

pub fn embed(stub: impl AsRef<Path>, output: impl AsRef<Path>, tap: &Tap) -> anyhow::Result<()> {
    let stub = stub.as_ref();
    let output = output.as_ref();
    let stub_bytes = fs::read(stub)?;
    let packaged = tap.embed_into(&stub_bytes)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
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
