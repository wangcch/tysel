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
