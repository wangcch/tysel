use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tysel_package::Tap;

#[test]
fn build_embeds_javascript_bundle_and_manifest() {
    let dir = std::env::temp_dir().join(format!("tysel-cli-build-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"ok\"); } };\n",
    )
    .unwrap();
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = dir.join("dist").join("hello-service");

    let result = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Type check       skipped"), "{stdout}");
    assert!(stdout.contains("Bundle           "), "{stdout}");
    assert!(stdout.contains("Capabilities     sqlite"), "{stdout}");
    assert!(stdout.contains("Runtime          service"), "{stdout}");
    assert!(stdout.contains("Output           "), "{stdout}");

    let tap = Tap::from_path(&output).expect("extract tap");
    assert!(fs::read(&output).unwrap().starts_with(b"stub-runtime"));
    assert_eq!(tap.manifest.application_id, "hello-service");
    assert_eq!(tap.manifest.listen, "127.0.0.1:0");
    assert!(tap.bundle_source().unwrap().contains("export default"));
    let origin = tap.parsed_source_map().unwrap().original_position(1, 1).unwrap();
    assert!(origin.source.ends_with("src/index.js"));
    assert!(!sidecar(&output, ".evidence.json").exists());
    assert!(!sidecar(&output, ".compat.json").exists());
    assert!(!sidecar(&output, ".sha256").exists());
    assert!(!sidecar(&output, ".sbom.cdx.json").exists());
    assert!(!sidecar(&output, ".licenses.json").exists());
}

#[test]
fn release_build_writes_compatible_deterministic_evidence() {
    let dir = temp_js_app("release-evidence");
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = dir.join("dist/release-app");
    let result = Command::new(cli_exe())
        .args([
            "build",
            "--release",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("run release build");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Checksum         "), "{stdout}");
    assert!(stdout.contains("Compatibility    "), "{stdout}");
    assert!(stdout.contains("SBOM             "), "{stdout}");
    assert!(stdout.contains("Licenses         "), "{stdout}");
    assert!(stdout.contains("Evidence         "), "{stdout}");

    let artifact = fs::read(&output).unwrap();
    let expected_digest = tysel_package::bundle_hash(&artifact);
    assert_eq!(
        fs::read_to_string(sidecar(&output, ".sha256")).unwrap(),
        format!("{expected_digest}\n")
    );
    let compatibility: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar(&output, ".compat.json")).unwrap()).unwrap();
    assert_eq!(compatibility["report_version"], 1);
    assert_eq!(compatibility["compatible"], true);
    assert_eq!(compatibility["status"], "current");
    assert!(compatibility.get("timestamp").is_none());

    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar(&output, ".evidence.json")).unwrap()).unwrap();
    assert_eq!(evidence["evidence_version"], 2);
    assert_eq!(evidence["artifact"]["sha256"], expected_digest);
    assert_eq!(evidence["artifact"]["size_bytes"], artifact.len());
    assert_eq!(evidence["application_id"], "hello-service");
    assert_eq!(evidence["compatibility"]["compatible"], true);
    assert!(evidence.get("timestamp").is_none());
    let sbom: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar(&output, ".sbom.cdx.json")).unwrap()).unwrap();
    assert_eq!(sbom["bomFormat"], "CycloneDX");
    assert_eq!(sbom["specVersion"], "1.5");
    assert_eq!(sbom["metadata"]["component"]["hashes"][0]["content"], expected_digest);
    let licenses: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar(&output, ".licenses.json")).unwrap()).unwrap();
    assert!(licenses["components"].as_array().unwrap().len() > 100);
    tysel_build::verify_release_evidence(&output).unwrap();
}

#[test]
fn release_commands_sign_and_verify_against_a_trust_policy() {
    let dir = temp_js_app("release-signature");
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let artifact = dir.join("dist/release-app");
    let build = Command::new(cli_exe())
        .args([
            "build",
            "--release",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            artifact.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));

    let key = dir.join("release.key");
    fs::write(&key, format!("{}\n", "07".repeat(32))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let key_info = Command::new(cli_exe())
        .args(["release", "key-info", "--key", key.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(key_info.status.success(), "{}", String::from_utf8_lossy(&key_info.stderr));
    let info: serde_json::Value = serde_json::from_slice(&key_info.stdout).unwrap();
    let trust = dir.join("trust.json");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    fs::write(
        &trust,
        serde_json::to_vec_pretty(&serde_json::json!({
            "policy_version": 1,
            "issued_at_unix": now,
            "expires_at_unix": now + 3600,
            "keys": [{
                "key_id": info["key_id"],
                "algorithm": "ed25519",
                "public_key": info["public_key"],
                "status": "active",
                "valid_from_unix": 0
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let sign = Command::new(cli_exe())
        .args(["release", "sign", artifact.to_str().unwrap(), "--key", key.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(sign.status.success(), "{}", String::from_utf8_lossy(&sign.stderr));
    assert!(sidecar(&artifact, ".evidence.sig.json").exists());

    let verify = Command::new(cli_exe())
        .args(["release", "verify", artifact.to_str().unwrap(), "--trust", trust.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(verify.status.success(), "{}", String::from_utf8_lossy(&verify.stderr));
    assert!(String::from_utf8_lossy(&verify.stdout).contains("Verified"));
}

#[test]
fn build_validates_precompiles_and_embeds_a_component() {
    let dir = std::env::temp_dir().join(format!("tysel-cli-build-wasm-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "echo-component"
entry = "echo.wasm"
profile = "component"
"#,
    )
    .unwrap();
    fs::write(dir.join("echo.wasm"), echo_component()).unwrap();
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let packaged = dir.join("dist/echo-component");

    let result = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            packaged.to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Type check       not applicable (Wasm Component)"), "{stdout}");
    assert!(stdout.contains("Component       "), "{stdout}");

    let tap = Tap::from_path(packaged).unwrap();
    assert!(tap.bundle.is_empty());
    assert_eq!(tap.components.len(), 1);
    assert_eq!(tap.components[0].name, "echo-component");
    assert_eq!(tap.components[0].abi_version, "0.4.0");
    assert_eq!(tap.components[0].aot.len(), 1);
}

#[test]
fn build_strips_typescript_and_embeds_original_source() {
    let dir = std::env::temp_dir().join(format!("tysel-cli-build-ts-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
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
    fs::write(
        dir.join("src/index.ts"),
        r#"
export default {
  async fetch(request: Request): Promise<Response> {
    return new Response("ok");
  },
};
"#,
    )
    .unwrap();
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = dir.join("dist").join("hello-service");

    let status = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("run tysel build");
    assert!(status.success());

    let tap = Tap::from_path(&output).expect("extract tap");
    let bundle = tap.bundle_source().unwrap();
    assert!(bundle.contains("export default"));
    assert!(!bundle.contains("Promise<Response>"));
    let origin = tap.parsed_source_map().unwrap().original_position(1, 1).unwrap();
    assert!(origin.source.ends_with("src/index.ts"));
    assert!(origin.content.unwrap().contains("request: Request"));
}

#[test]
fn build_does_not_embed_postgres_urls() {
    let dir = std::env::temp_dir().join(format!("tysel-cli-build-pg-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[permissions]
postgres = ["main:read-write"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"ok\"); } };\n",
    )
    .unwrap();
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = dir.join("dist").join("hello-service");

    let result = Command::new(cli_exe())
        .env("TYSEL_POSTGRES_MAIN", "postgres://tysel:s3cret-not-for-tap@127.0.0.1:5432/tysel")
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("postgres"), "{stdout}");

    let tap = Tap::from_path(&output).expect("extract tap");
    assert_eq!(tap.manifest.postgres, ["main:read-write"]);
    let bytes = fs::read(&output).unwrap();
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(haystack.contains("main:read-write"), "alias missing from binary");
    assert!(!haystack.contains("postgres://"), "URL leaked into binary");
    assert!(!haystack.contains("s3cret-not-for-tap"), "password leaked into binary");
}

#[test]
fn build_rejects_a_cross_compile_target() {
    let dir = temp_js_app("build-target");
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--target",
            "linux-riscv64",
            "--output",
            dir.join("dist/app").to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cross-compilation is not implemented"), "{stderr}");
}

#[test]
fn build_rejects_a_mismatched_profile() {
    let dir = temp_js_app("build-profile");
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();
    let output = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
            "--profile",
            "worker",
            "--output",
            dir.join("dist/app").to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not match manifest profile service"), "{stderr}");
}

#[test]
fn build_fails_when_the_stub_is_missing() {
    let dir = temp_js_app("build-missing-stub");
    let output = Command::new(cli_exe())
        .args([
            "build",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            dir.join("missing-stub").to_str().unwrap(),
            "--output",
            dir.join("dist/app").to_str().unwrap(),
        ])
        .output()
        .expect("run tysel build");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("runtime stub not found"), "{stderr}");
}

fn temp_js_app(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tysel-cli-{name}-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"ok\"); } };\n",
    )
    .unwrap();
    dir
}

fn echo_component() -> Vec<u8> {
    wat::parse_str(
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
    .unwrap()
}

fn cli_exe() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_tysel") {
        return PathBuf::from(path);
    }
    let test_exe = std::env::current_exe().expect("current_exe");
    let mut candidate = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("tysel"))
        .expect("target debug directory");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    assert!(candidate.is_file(), "missing tysel at {}", candidate.display());
    candidate
}

fn sidecar(output: &std::path::Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", output.display()))
}
