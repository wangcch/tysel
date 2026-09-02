use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tysel_manifest::Manifest;
use tysel_package::Tap;

fn write_release_stub(path: &std::path::Path) {
    let mut stub = b"stub-runtime".to_vec();
    stub.extend_from_slice(include_bytes!("../../tysel-build/src/runtime-components.json"));
    fs::write(path, stub).unwrap();
}

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
    write_release_stub(&stub);
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
    write_release_stub(&stub);
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

    let sign_artifact = Command::new(cli_exe())
        .args([
            "release",
            "sign-artifact",
            artifact.to_str().unwrap(),
            "--target",
            "linux-x64",
            "--key",
            key.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(sign_artifact.status.success(), "{}", String::from_utf8_lossy(&sign_artifact.stderr));
    assert!(sidecar(&artifact, ".sig.json").exists());
    let verify_artifact = Command::new(cli_exe())
        .args([
            "release",
            "verify-artifact",
            artifact.to_str().unwrap(),
            "--trust",
            trust.to_str().unwrap(),
            "--target",
            "linux-x64",
        ])
        .output()
        .unwrap();
    assert!(
        verify_artifact.status.success(),
        "{}",
        String::from_utf8_lossy(&verify_artifact.stderr)
    );
    assert!(String::from_utf8_lossy(&verify_artifact.stdout).contains("linux-x64"));
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
    assert!(stdout.contains("Capabilities     none"), "{stdout}");

    let tap = Tap::from_path(packaged).unwrap();
    assert!(tap.bundle.is_empty());
    assert_eq!(tap.components.len(), 1);
    assert_eq!(tap.components[0].name, "echo-component");
    assert_eq!(tap.components[0].abi_version, "0.4.0");
    assert_eq!(tap.components[0].aot.len(), 1);
}

#[test]
fn build_rejects_a_wasm_override_for_a_service_profile() {
    let dir = temp_js_app("build-component-profile-mismatch");
    let component = dir.join("override.wasm");
    fs::write(&component, echo_component()).unwrap();
    let stub = dir.join("tysel-service");
    fs::write(&stub, b"stub-runtime").unwrap();

    let output = Command::new(cli_exe())
        .args([
            "build",
            component.to_str().unwrap(),
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--stub",
            stub.to_str().unwrap(),
        ])
        .output()
        .expect("build with mismatched Component override");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(".wasm entry requires app.profile = \"component\"")
    );
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
fn build_does_not_embed_database_urls() {
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
redis = ["cache:read-write"]
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
        .env("TYSEL_REDIS_CACHE", "redis://:redis-secret-not-for-tap@127.0.0.1:6379/0")
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
    assert!(stdout.contains("redis"), "{stdout}");

    let tap = Tap::from_path(&output).expect("extract tap");
    assert_eq!(tap.manifest.postgres, ["main:read-write"]);
    assert_eq!(tap.manifest.redis, ["cache:read-write"]);
    let bytes = fs::read(&output).unwrap();
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(haystack.contains("main:read-write"), "alias missing from binary");
    assert!(!haystack.contains("postgres://"), "URL leaked into binary");
    assert!(!haystack.contains("s3cret-not-for-tap"), "password leaked into binary");
    assert!(haystack.contains("cache:read-write"), "Redis alias missing from binary");
    assert!(!haystack.contains("redis://"), "Redis URL leaked into binary");
    assert!(!haystack.contains("redis-secret-not-for-tap"), "Redis password leaked into binary");
}

#[test]
fn build_rejects_an_unsupported_target() {
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
    assert!(stderr.contains("unsupported build target linux-riscv64"), "{stderr}");
}

#[test]
fn image_generates_a_nonroot_distroless_context() {
    let dir = temp_js_app("image-context");
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "image-app"
entry = "src/index.js"
profile = "service"

[server]
listen = "0.0.0.0:8080"
"#,
    )
    .unwrap();
    let binary = dir.join("linux-app");
    let binary_bytes = fake_tysel_elf(62, "image-app", "service", "0.0.0.0:8080");
    fs::write(&binary, &binary_bytes).unwrap();
    let context = dir.join("dist/container");

    let output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--image-version",
            "1.4.0",
            "--label",
            "org.opencontainers.image.source=https://example.invalid/tysel",
            "--label",
            "example.literal=$PATH",
            "--output-dir",
            context.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(context.join("tysel-app")).unwrap(), binary_bytes);
    let dockerfile = fs::read_to_string(context.join("Dockerfile")).unwrap();
    assert!(dockerfile.contains("FROM gcr.io/distroless/cc-debian13:nonroot"));
    assert!(dockerfile.contains("USER 65532:65532"));
    assert!(dockerfile.contains("EXPOSE 8080"));
    assert!(dockerfile.contains("ENTRYPOINT [\"/app/tysel-app\"]"));
    assert!(dockerfile.contains("org.opencontainers.image.title=\"image-app\""));
    assert!(dockerfile.contains("org.opencontainers.image.version=\"1.4.0\""));
    assert!(
        dockerfile.contains("org.opencontainers.image.source=\"https://example.invalid/tysel\"")
    );
    assert!(dockerfile.contains("example.literal=\"\\$PATH\""));
    assert!(dockerfile.contains("io.tysel.artifact.digest=\"sha256:"));

    let repeated = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            context.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("refusing to overwrite"));
}

#[test]
fn image_rejects_manifest_that_differs_from_embedded_tap() {
    let dir = temp_js_app("image-embedded-listen");
    let manifest =
        fs::read_to_string(dir.join("tysel.toml")).unwrap().replace("127.0.0.1:0", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let binary = dir.join("linux-app");
    fs::write(&binary, fake_tysel_elf(62, "hello-service", "service", "0.0.0.0:9090")).unwrap();
    let output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/container").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("embeds listen '0.0.0.0:9090'"), "{stderr}");
    assert!(stderr.contains("declares '0.0.0.0:8080'"), "{stderr}");
}

#[test]
fn image_rejects_component_profile_with_deployment_doc() {
    let dir = temp_js_app("image-component");
    let manifest = fs::read_to_string(dir.join("tysel.toml"))
        .unwrap()
        .replace("entry = \"src/index.js\"", "entry = \"echo.wasm\"")
        .replace("profile = \"service\"", "profile = \"component\"");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let output = Command::new(cli_exe())
        .args(["image", "--context-only", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not support profile = \"component\""), "{stderr}");
    assert!(stderr.contains("docs/operations/component-tasks.md"), "{stderr}");
}

#[test]
fn image_copies_only_verified_release_sidecars() {
    let dir = temp_js_app("image-sidecars");
    let manifest =
        fs::read_to_string(dir.join("tysel.toml")).unwrap().replace("127.0.0.1:0", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let binary = dir.join("release-app");
    fs::write(&binary, fake_release_tysel_elf(62, "hello-service", "service", "0.0.0.0:8080"))
        .unwrap();
    tysel_build::write_release_evidence(&binary, "linux-x64").unwrap();
    let context = dir.join("dist/container");
    let output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--copy-sidecars",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            context.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    for suffix in [".sha256", ".compat.json", ".sbom.cdx.json", ".licenses.json", ".evidence.json"]
    {
        assert!(context.join(format!("tysel-app{suffix}")).is_file());
    }
    let dockerfile = fs::read_to_string(context.join("Dockerfile")).unwrap();
    assert!(!dockerfile.contains("evidence.json"));

    let replaced = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--force",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            context.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(replaced.status.success(), "{}", String::from_utf8_lossy(&replaced.stderr));
    for suffix in [".sha256", ".compat.json", ".sbom.cdx.json", ".licenses.json", ".evidence.json"]
    {
        assert!(!context.join(format!("tysel-app{suffix}")).exists());
    }
}

#[test]
fn image_rejects_release_evidence_for_another_target() {
    let dir = temp_js_app("image-sidecar-target");
    let manifest =
        fs::read_to_string(dir.join("tysel.toml")).unwrap().replace("127.0.0.1:0", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let binary = dir.join("release-app");
    fs::write(&binary, fake_release_tysel_elf(62, "hello-service", "service", "0.0.0.0:8080"))
        .unwrap();
    tysel_build::write_release_evidence(&binary, "linux-arm64").unwrap();

    let output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--copy-sidecars",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/container").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("records target 'linux-arm64'"), "{stderr}");
    assert!(stderr.contains("requires 'linux-x64'"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn image_uses_docker_environment_as_builder_executable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_js_app("image-builder");
    let manifest =
        fs::read_to_string(dir.join("tysel.toml")).unwrap().replace("127.0.0.1:0", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let binary = dir.join("linux-app");
    fs::write(&binary, fake_tysel_elf(62, "hello-service", "service", "0.0.0.0:8080")).unwrap();
    let builder = dir.join("podman-fixture");
    fs::write(&builder, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$BUILDER_LOG\"\n").unwrap();
    let mut permissions = fs::metadata(&builder).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&builder, permissions).unwrap();
    let log = dir.join("builder.log");
    let output = Command::new(cli_exe())
        .args([
            "image",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/container").to_str().unwrap(),
        ])
        .env("DOCKER", &builder)
        .env("BUILDER_LOG", &log)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let arguments = fs::read_to_string(&log).unwrap();
    assert!(arguments.contains("build\n--platform\nlinux/amd64\n--tag\nhello-service:latest"));

    let ignored_builder = dir.join("ignored-builder");
    fs::write(&ignored_builder, "#!/bin/sh\nexit 91\n").unwrap();
    let mut permissions = fs::metadata(&ignored_builder).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&ignored_builder, permissions).unwrap();
    let output = Command::new(cli_exe())
        .args([
            "image",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--builder",
            builder.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/explicit-builder").to_str().unwrap(),
        ])
        .env("DOCKER", ignored_builder)
        .env("BUILDER_LOG", &log)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn image_rejects_loopback_listeners() {
    let dir = temp_js_app("image-loopback");
    let binary = dir.join("linux-app");
    fs::write(&binary, fake_linux_elf(62)).unwrap();
    let output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/container").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("listen on 0.0.0.0"));
}

#[test]
fn image_rejects_truncated_elf_and_non_wildcard_listeners() {
    let dir = temp_js_app("image-invalid-elf");
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "image-app"
entry = "src/index.js"
profile = "service"

[server]
listen = "192.0.2.10:8080"
"#,
    )
    .unwrap();
    let binary = dir.join("linux-app");
    fs::write(&binary, b"\x7fELF").unwrap();
    let manifest_path = dir.join("tysel.toml");
    let output_dir = dir.join("dist/container");
    let arguments = [
        "image",
        "--context-only",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--binary",
        binary.to_str().unwrap(),
        "--output-dir",
        output_dir.to_str().unwrap(),
    ];
    let listener = Command::new(cli_exe()).args(arguments).output().unwrap();
    assert!(!listener.status.success());
    assert!(String::from_utf8_lossy(&listener.stderr).contains("0.0.0.0 or [::]"));

    let manifest = fs::read_to_string(dir.join("tysel.toml"))
        .unwrap()
        .replace("192.0.2.10:8080", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let elf = Command::new(cli_exe()).args(arguments).output().unwrap();
    assert!(!elf.status.success());
    assert!(String::from_utf8_lossy(&elf.stderr).contains("not a Linux ELF executable"));
}

#[test]
fn image_allows_musl_only_with_an_explicit_custom_base() {
    let dir = temp_js_app("image-custom-musl-base");
    let manifest =
        fs::read_to_string(dir.join("tysel.toml")).unwrap().replace("127.0.0.1:0", "0.0.0.0:8080");
    fs::write(dir.join("tysel.toml"), manifest).unwrap();
    let binary = dir.join("musl-app");
    fs::write(
        &binary,
        fake_tysel_elf_with_interpreter(
            62,
            "hello-service",
            "service",
            "0.0.0.0:8080",
            "/lib/ld-musl-x86_64.so.1",
        ),
    )
    .unwrap();

    let default_output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--output-dir",
            dir.join("dist/default-base").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!default_output.status.success());
    assert!(
        String::from_utf8_lossy(&default_output.stderr)
            .contains("incompatible with the default glibc")
    );

    let custom_output = Command::new(cli_exe())
        .args([
            "image",
            "--context-only",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
            "--base-image",
            "registry.example/runtime-musl:1",
            "--output-dir",
            dir.join("dist/custom-base").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(custom_output.status.success(), "{}", String::from_utf8_lossy(&custom_output.stderr));
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

fn fake_linux_elf(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0_u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes
}

fn fake_tysel_elf(machine: u16, name: &str, profile: &str, listen: &str) -> Vec<u8> {
    let manifest = Manifest::parse(&format!(
        r#"
[app]
name = "{name}"
entry = "src/index.js"
profile = "{profile}"

[server]
listen = "{listen}"
"#
    ))
    .unwrap();
    let tap =
        tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), Vec::new(), Vec::new());
    tap.embed_into(&fake_linux_elf(machine)).unwrap()
}

fn fake_tysel_elf_with_interpreter(
    machine: u16,
    name: &str,
    profile: &str,
    listen: &str,
    interpreter: &str,
) -> Vec<u8> {
    let mut elf = fake_linux_elf(machine);
    let mut value = interpreter.as_bytes().to_vec();
    value.push(0);
    let string_offset = elf.len() + 56;
    elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
    elf[56..58].copy_from_slice(&1_u16.to_le_bytes());
    elf.resize(string_offset, 0);
    elf[64..68].copy_from_slice(&3_u32.to_le_bytes());
    elf[72..80].copy_from_slice(&(string_offset as u64).to_le_bytes());
    elf[96..104].copy_from_slice(&(value.len() as u64).to_le_bytes());
    elf.extend_from_slice(&value);

    let manifest = Manifest::parse(&format!(
        r#"
[app]
name = "{name}"
entry = "src/index.js"
profile = "{profile}"

[server]
listen = "{listen}"
"#
    ))
    .unwrap();
    let tap =
        tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), Vec::new(), Vec::new());
    tap.embed_into(&elf).unwrap()
}

fn fake_release_tysel_elf(machine: u16, name: &str, profile: &str, listen: &str) -> Vec<u8> {
    let manifest = Manifest::parse(&format!(
        r#"
[app]
name = "{name}"
entry = "src/index.js"
profile = "{profile}"

[server]
listen = "{listen}"
"#
    ))
    .unwrap();
    let tap =
        tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), Vec::new(), Vec::new());
    let mut stub = fake_linux_elf(machine);
    stub.extend_from_slice(include_bytes!("../../tysel-build/src/runtime-components.json"));
    tap.embed_into(&stub).unwrap()
}

fn sidecar(output: &std::path::Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", output.display()))
}
