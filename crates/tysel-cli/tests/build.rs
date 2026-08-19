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
