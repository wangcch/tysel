//! TypeScript type-check, transpile, bundle, and native executable assembly.
//!
//! Spike C copies a prebuilt runtime stub and appends a TAP trailer containing
//! the ESM bundle, embedded manifest, and source map.

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
    match entry.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "js" | "mjs" => {
            let text = std::str::from_utf8(&source)
                .map_err(|_| anyhow::anyhow!("JavaScript bundle must be utf-8"))?;
            let map = identity_source_map(&display, text)?;
            Ok((source, map))
        }
        "ts" | "mts" => {
            let text = std::str::from_utf8(&source)
                .map_err(|_| anyhow::anyhow!("TypeScript source must be utf-8"))?;
            transpile::transpile_typescript(entry, text)
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
}
