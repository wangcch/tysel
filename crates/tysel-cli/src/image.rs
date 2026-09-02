use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use tysel_build::verify_release_evidence;
use tysel_manifest::Manifest;
use tysel_package::{Tap, bundle_hash};

const APP_FILE: &str = "tysel-app";
const DOCKERFILE: &str = "Dockerfile";
const DEFAULT_BASE_IMAGE: &str = "gcr.io/distroless/cc-debian13:nonroot";
const SIDECAR_SUFFIXES: [&str; 5] =
    [".sha256", ".compat.json", ".sbom.cdx.json", ".licenses.json", ".evidence.json"];
const COMPONENT_TASKS_DOC: &str = "docs/operations/component-tasks.md";

pub struct Options {
    pub entry: Option<PathBuf>,
    pub manifest: PathBuf,
    pub binary: Option<PathBuf>,
    pub stub: Option<PathBuf>,
    pub tag: Option<String>,
    pub output_dir: PathBuf,
    pub base_image: String,
    pub builder: Option<PathBuf>,
    pub copy_sidecars: bool,
    pub image_version: Option<String>,
    pub labels: Vec<String>,
    pub context_only: bool,
    pub force: bool,
}

pub fn run(options: Options) -> Result<()> {
    validate_base_image(&options.base_image)?;
    let enforce_default_glibc = options.base_image == DEFAULT_BASE_IMAGE;
    let manifest = Manifest::from_path(&options.manifest)
        .with_context(|| format!("failed to read {}", options.manifest.display()))?;
    if manifest.app.profile == "component" {
        return Err(anyhow!(
            "tysel image does not support profile = \"component\"; see {COMPONENT_TASKS_DOC}"
        ));
    }
    let port = container_port(&manifest.server.listen)?;
    let tag = options.tag.unwrap_or_else(|| format!("{}:latest", manifest.app.name));
    if tag.is_empty() || tag.chars().any(char::is_whitespace) {
        return Err(anyhow!("container tag must be non-empty and contain no whitespace"));
    }

    let supplied_binary = if let Some(binary) = options.binary.as_deref() {
        let executable = validate_linux_executable(binary, enforce_default_glibc)?;
        let tap = Tap::from_path(binary).with_context(|| {
            format!("{} is not a Tysel executable with embedded TAP metadata", binary.display())
        })?;
        validate_embedded_manifest(binary, &manifest, &tap)?;
        if options.copy_sidecars {
            let evidence = verify_release_evidence(binary).with_context(|| {
                format!("cannot copy unverified release sidecars for {}", binary.display())
            })?;
            validate_evidence_target(binary, executable.platform, &evidence.artifact.target)?;
        }
        Some(executable)
    } else if !cfg!(target_os = "linux") {
        return Err(anyhow!(
            "building a container requires a Linux executable; pass --binary from a Linux build"
        ));
    } else {
        None
    };

    fs::create_dir_all(&options.output_dir)
        .with_context(|| format!("create {}", options.output_dir.display()))?;
    let app_output = options.output_dir.join(APP_FILE);
    let dockerfile = options.output_dir.join(DOCKERFILE);
    let sidecar_outputs = generated_sidecar_paths(&options.output_dir);
    let mut generated = vec![app_output.clone(), dockerfile.clone()];
    generated.extend(sidecar_outputs.iter().cloned());
    preflight_generated(&generated, options.force)?;
    if options.force && !options.copy_sidecars {
        remove_generated_sidecars(&sidecar_outputs)?;
    }

    let executable = if let Some(binary) = options.binary.as_deref() {
        fs::copy(binary, &app_output).with_context(|| {
            format!("copy Linux executable {} to {}", binary.display(), app_output.display())
        })?;
        set_executable(&app_output)?;
        if options.copy_sidecars {
            copy_sidecars(binary, &options.output_dir)?;
        }
        supplied_binary.expect("validated supplied binary")
    } else {
        super::build::run(
            options.manifest.clone(),
            options.entry.clone(),
            options.stub.clone(),
            Some(app_output.clone()),
            None,
            None,
            true,
        )?;
        validate_linux_executable(&app_output, enforce_default_glibc)?
    };

    let tap =
        Tap::from_path(&app_output).context("generated executable contains an invalid TAP")?;
    let artifact =
        fs::read(&app_output).with_context(|| format!("read {}", app_output.display()))?;
    let labels = image_labels(
        &tap,
        &bundle_hash(&artifact),
        options.image_version.as_deref(),
        &options.labels,
    )?;
    let label_text = labels
        .iter()
        .map(|(key, value)| format!("LABEL {key}=\"{}\"\n", escape_label_value(value)))
        .collect::<String>();

    let dockerfile_text = format!(
        "FROM {}\n{}WORKDIR /app\nCOPY --chown=65532:65532 {} /app/{}\nUSER 65532:65532\nEXPOSE {}\nENTRYPOINT [\"/app/{}\"]\n",
        options.base_image, label_text, APP_FILE, APP_FILE, port, APP_FILE
    );
    fs::write(&dockerfile, dockerfile_text)
        .with_context(|| format!("write {}", dockerfile.display()))?;
    println!("Context          {}", options.output_dir.display());
    println!("Base             {}", options.base_image);
    println!("User             65532:65532");
    println!("Port             {port}");
    println!("Platform         {}", executable.platform);
    println!("Artifact digest  sha256:{}", bundle_hash(&artifact));

    if options.context_only {
        println!("Image            skipped (--context-only)");
        return Ok(());
    }
    let builder = builder_command(options.builder)?;
    let status = Command::new(&builder)
        .args(["build", "--platform", executable.platform, "--tag", &tag])
        .arg(&options.output_dir)
        .status()
        .with_context(|| {
            format!(
                "failed to start container builder '{}'; use --context-only to only generate the context",
                builder.to_string_lossy()
            )
        })?;
    if !status.success() {
        return Err(anyhow!(
            "container builder '{}' failed with status {status}",
            builder.to_string_lossy()
        ));
    }
    println!("Image            {tag}");
    Ok(())
}

fn preflight_generated(paths: &[PathBuf], force: bool) -> Result<()> {
    let conflicts: Vec<_> = paths.iter().filter(|path| path.exists()).collect();
    if !force && !conflicts.is_empty() {
        let names = conflicts.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        return Err(anyhow!(
            "refusing to overwrite generated files: {} (pass --force)",
            names.join(", ")
        ));
    }
    Ok(())
}

fn validate_embedded_manifest(path: &Path, manifest: &Manifest, tap: &Tap) -> Result<()> {
    let embedded = &tap.manifest;
    if embedded.execution_profile == "component" {
        return Err(anyhow!(
            "{} embeds profile = \"component\"; tysel image does not support Component tasks; see {COMPONENT_TASKS_DOC}",
            path.display()
        ));
    }
    if embedded.application_id != manifest.app.name {
        return Err(anyhow!(
            "{} embeds application '{}', but the selected manifest declares '{}'; rebuild with the selected manifest",
            path.display(),
            embedded.application_id,
            manifest.app.name
        ));
    }
    if embedded.execution_profile != manifest.app.profile {
        return Err(anyhow!(
            "{} embeds profile '{}', but the selected manifest declares '{}'",
            path.display(),
            embedded.execution_profile,
            manifest.app.profile
        ));
    }
    container_port(&embedded.listen).with_context(|| {
        format!("{} embeds a listener that cannot be used in a container", path.display())
    })?;
    if embedded.listen != manifest.server.listen {
        return Err(anyhow!(
            "{} embeds listen '{}', but the selected manifest declares '{}'; rebuild the executable with the container manifest",
            path.display(),
            embedded.listen,
            manifest.server.listen
        ));
    }
    Ok(())
}

fn copy_sidecars(binary: &Path, output_dir: &Path) -> Result<()> {
    for suffix in SIDECAR_SUFFIXES {
        let mut source_name = binary.as_os_str().to_os_string();
        source_name.push(suffix);
        let source = PathBuf::from(source_name);
        let destination = output_dir.join(format!("{APP_FILE}{suffix}"));
        fs::copy(&source, &destination).with_context(|| {
            format!("copy verified sidecar {} to {}", source.display(), destination.display())
        })?;
    }
    Ok(())
}

fn generated_sidecar_paths(output_dir: &Path) -> Vec<PathBuf> {
    SIDECAR_SUFFIXES.iter().map(|suffix| output_dir.join(format!("{APP_FILE}{suffix}"))).collect()
}

fn remove_generated_sidecars(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("remove stale {}", path.display()));
            }
        }
    }
    Ok(())
}

fn validate_evidence_target(path: &Path, platform: &str, target: &str) -> Result<()> {
    let expected = match platform {
        "linux/amd64" => "linux-x64",
        "linux/arm64" => "linux-arm64",
        _ => return Err(anyhow!("unsupported container platform '{platform}'")),
    };
    if target != expected {
        return Err(anyhow!(
            "{} release evidence records target '{}', but its ELF platform requires '{}'",
            path.display(),
            target,
            expected
        ));
    }
    Ok(())
}

fn builder_command(explicit: Option<PathBuf>) -> Result<OsString> {
    let builder = explicit
        .map(PathBuf::into_os_string)
        .or_else(|| env::var_os("DOCKER"))
        .unwrap_or_else(|| OsString::from("docker"));
    if builder.is_empty() {
        return Err(anyhow!("container builder executable must not be empty"));
    }
    Ok(builder)
}

fn image_labels(
    tap: &Tap,
    artifact_sha256: &str,
    image_version: Option<&str>,
    custom: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut labels = BTreeMap::from([
        ("io.tysel.artifact.digest".into(), format!("sha256:{artifact_sha256}")),
        ("io.tysel.execution-profile".into(), tap.manifest.execution_profile.clone()),
        ("io.tysel.runtime.version".into(), tap.manifest.runtime_version.clone()),
        ("org.opencontainers.image.title".into(), tap.manifest.application_id.clone()),
    ]);
    if let Some(version) = image_version {
        validate_label_value(version)?;
        labels.insert("org.opencontainers.image.version".into(), version.into());
    }
    for label in custom {
        let (key, value) = label
            .split_once('=')
            .ok_or_else(|| anyhow!("image label must use KEY=VALUE: '{label}'"))?;
        validate_label_key(key)?;
        validate_label_value(value)?;
        if labels.contains_key(key) {
            return Err(anyhow!(
                "image label '{key}' is generated by Tysel and cannot be overridden"
            ));
        }
        labels.insert(key.into(), value.into());
    }
    Ok(labels)
}

fn validate_label_key(key: &str) -> Result<()> {
    if key.is_empty()
        || !key
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !key.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b'/')
        })
    {
        return Err(anyhow!("invalid image label key '{key}'"));
    }
    Ok(())
}

fn validate_label_value(value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(anyhow!("image label values must not contain control characters"));
    }
    Ok(())
}

fn escape_label_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('$', "\\$").replace('"', "\\\"")
}

#[derive(Clone, Copy)]
struct LinuxExecutable {
    platform: &'static str,
}

fn validate_linux_executable(path: &Path, enforce_default_glibc: bool) -> Result<LinuxExecutable> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < 64 || !bytes.starts_with(b"\x7fELF") {
        return Err(anyhow!("{} is not a Linux ELF executable", path.display()));
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err(anyhow!(
            "{} must be a 64-bit little-endian Linux ELF executable",
            path.display()
        ));
    }
    let elf_type = read_u16(&bytes, 16)?;
    if !matches!(elf_type, 2 | 3) {
        return Err(anyhow!("{} is not an executable or PIE ELF file", path.display()));
    }
    let platform = match read_u16(&bytes, 18)? {
        62 => "linux/amd64",
        183 => "linux/arm64",
        machine => {
            return Err(anyhow!(
                "{} uses unsupported ELF machine {machine}; expected x86_64 or aarch64",
                path.display()
            ));
        }
    };
    validate_interpreter(path, &bytes, enforce_default_glibc)?;
    Ok(LinuxExecutable { platform })
}

fn validate_interpreter(path: &Path, bytes: &[u8], enforce_default_glibc: bool) -> Result<()> {
    let program_offset = usize::try_from(read_u64(bytes, 32)?)
        .map_err(|_| anyhow!("{} has an invalid ELF program table", path.display()))?;
    let entry_size = usize::from(read_u16(bytes, 54)?);
    let entry_count = usize::from(read_u16(bytes, 56)?);
    if entry_count > 0 && entry_size < 56 {
        return Err(anyhow!("{} has an invalid ELF program header size", path.display()));
    }
    for index in 0..entry_count {
        let offset = program_offset
            .checked_add(index.saturating_mul(entry_size))
            .ok_or_else(|| anyhow!("{} has an invalid ELF program table", path.display()))?;
        if read_u32(bytes, offset)? != 3 {
            continue;
        }
        let string_offset = usize::try_from(read_u64(bytes, offset + 8)?)
            .map_err(|_| anyhow!("{} has an invalid ELF interpreter", path.display()))?;
        let string_len = usize::try_from(read_u64(bytes, offset + 32)?)
            .map_err(|_| anyhow!("{} has an invalid ELF interpreter", path.display()))?;
        let end = string_offset
            .checked_add(string_len)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| anyhow!("{} has a truncated ELF interpreter", path.display()))?;
        let interpreter = String::from_utf8_lossy(&bytes[string_offset..end]);
        if enforce_default_glibc && !interpreter.contains("ld-linux") {
            return Err(anyhow!(
                "{} uses interpreter '{}', incompatible with the default glibc distroless image",
                path.display(),
                interpreter.trim_end_matches('\0')
            ));
        }
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes.get(offset..offset + 2).ok_or_else(|| anyhow!("truncated ELF header"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes.get(offset..offset + 4).ok_or_else(|| anyhow!("truncated ELF header"))?;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let value = bytes.get(offset..offset + 8).ok_or_else(|| anyhow!("truncated ELF header"))?;
    Ok(u64::from_le_bytes(value.try_into().expect("eight bytes")))
}

fn validate_base_image(image: &str) -> Result<()> {
    if image.is_empty()
        || !image
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '/' | ':' | '@' | '_' | '-'))
    {
        return Err(anyhow!("invalid base image reference '{image}'"));
    }
    Ok(())
}

fn container_port(listen: &str) -> Result<u16> {
    let address: SocketAddr = listen.parse().map_err(|_| {
        anyhow!("container listen address must be an IP address and port: '{listen}'")
    })?;
    if !address.ip().is_unspecified() {
        return Err(anyhow!(
            "container services must listen on 0.0.0.0 or [::]; update [server].listen (currently '{listen}')"
        ));
    }
    if address.port() == 0 {
        return Err(anyhow!("container listen address must use a non-zero port: '{listen}'"));
    }
    Ok(address.port())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf_with_interpreter(interpreter: Option<&str>) -> Vec<u8> {
        let entry_count = usize::from(interpreter.is_some());
        let mut bytes = vec![0_u8; 64 + entry_count * 56];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
        bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
        bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&(entry_count as u16).to_le_bytes());
        if let Some(interpreter) = interpreter {
            let mut value = interpreter.as_bytes().to_vec();
            value.push(0);
            let string_offset = bytes.len();
            bytes[64..68].copy_from_slice(&3_u32.to_le_bytes());
            bytes[72..80].copy_from_slice(&(string_offset as u64).to_le_bytes());
            bytes[96..104].copy_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&value);
        }
        bytes
    }

    #[test]
    fn glibc_interpreter_is_accepted() {
        validate_interpreter(
            Path::new("glibc-app"),
            &elf_with_interpreter(Some("/lib64/ld-linux-x86-64.so.2")),
            true,
        )
        .unwrap();
    }

    #[test]
    fn musl_interpreter_is_rejected_for_default_base() {
        let error = validate_interpreter(
            Path::new("musl-app"),
            &elf_with_interpreter(Some("/lib/ld-musl-x86_64.so.1")),
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("incompatible with the default glibc"));
    }

    #[test]
    fn static_elf_without_interpreter_is_accepted() {
        validate_interpreter(Path::new("static-app"), &elf_with_interpreter(None), true).unwrap();
    }

    #[test]
    fn custom_base_accepts_a_structurally_valid_musl_interpreter() {
        validate_interpreter(
            Path::new("musl-app"),
            &elf_with_interpreter(Some("/lib/ld-musl-x86_64.so.1")),
            false,
        )
        .unwrap();
    }

    #[test]
    fn generated_labels_cannot_be_overridden() {
        let tap = Tap::new(
            tysel_package::PackageManifest {
                format_version: 0,
                runtime_version: "1.2.3".into(),
                application_id: "orders".into(),
                entrypoint: "src/index.ts".into(),
                execution_profile: "service".into(),
                listen: "0.0.0.0:8080".into(),
                memory_limit_bytes: 1,
                cpu_ms_per_turn: 1,
                request_timeout_ms: 1,
                bundle_hash: String::new(),
                max_request_bytes: 1,
                max_response_bytes: 1,
                websocket: false,
                workers: 1,
                max_in_flight: 1,
                http1: true,
                http2: false,
                sqlite_path: String::new(),
                secret_names: Vec::new(),
                fetch_hosts: Vec::new(),
                postgres: Vec::new(),
                redis: Vec::new(),
                fs_read: Vec::new(),
                fs_write: Vec::new(),
                json_logs: true,
            },
            Vec::new(),
            Vec::new(),
        );
        let error = image_labels(&tap, "00", None, &["io.tysel.artifact.digest=changed".into()])
            .unwrap_err();
        assert!(error.to_string().contains("cannot be overridden"));
    }

    #[test]
    fn label_values_preserve_dollar_signs() {
        assert_eq!(escape_label_value("$PATH ${HOME}"), "\\$PATH \\${HOME}");
    }
}
