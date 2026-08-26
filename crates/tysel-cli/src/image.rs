use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use tysel_manifest::Manifest;

const APP_FILE: &str = "tysel-app";
const DOCKERFILE: &str = "Dockerfile";

pub struct Options {
    pub entry: Option<PathBuf>,
    pub manifest: PathBuf,
    pub binary: Option<PathBuf>,
    pub stub: Option<PathBuf>,
    pub tag: Option<String>,
    pub output_dir: PathBuf,
    pub base_image: String,
    pub context_only: bool,
    pub force: bool,
}

pub fn run(options: Options) -> Result<()> {
    validate_base_image(&options.base_image)?;
    let manifest = Manifest::from_path(&options.manifest)
        .with_context(|| format!("failed to read {}", options.manifest.display()))?;
    let port = container_port(&manifest.server.listen)?;
    let tag = options.tag.unwrap_or_else(|| format!("{}:latest", manifest.app.name));
    if tag.is_empty() || tag.chars().any(char::is_whitespace) {
        return Err(anyhow!("container tag must be non-empty and contain no whitespace"));
    }

    let supplied_binary = if let Some(binary) = options.binary.as_deref() {
        Some(validate_linux_executable(binary)?)
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
    preflight_generated(&[&app_output, &dockerfile], options.force)?;

    let executable = if let Some(binary) = options.binary {
        fs::copy(&binary, &app_output).with_context(|| {
            format!("copy Linux executable {} to {}", binary.display(), app_output.display())
        })?;
        set_executable(&app_output)?;
        supplied_binary.expect("validated supplied binary")
    } else {
        super::build::run(
            options.manifest,
            options.entry,
            options.stub,
            Some(app_output.clone()),
            None,
            None,
            true,
        )?;
        validate_linux_executable(&app_output)?
    };

    let dockerfile_text = format!(
        "FROM {}\nWORKDIR /app\nCOPY --chown=65532:65532 {} /app/{}\nUSER 65532:65532\nEXPOSE {}\nENTRYPOINT [\"/app/{}\"]\n",
        options.base_image, APP_FILE, APP_FILE, port, APP_FILE
    );
    fs::write(&dockerfile, dockerfile_text)
        .with_context(|| format!("write {}", dockerfile.display()))?;
    println!("Context          {}", options.output_dir.display());
    println!("Base             {}", options.base_image);
    println!("User             65532:65532");
    println!("Port             {port}");
    println!("Platform         {}", executable.platform);

    if options.context_only {
        println!("Image            skipped (--context-only)");
        return Ok(());
    }
    let status = Command::new("docker")
        .args(["build", "--platform", executable.platform, "--tag", &tag])
        .arg(&options.output_dir)
        .status()
        .context("failed to start Docker; use --context-only to only generate the context")?;
    if !status.success() {
        return Err(anyhow!("docker build failed with status {status}"));
    }
    println!("Image            {tag}");
    Ok(())
}

fn preflight_generated(paths: &[&Path], force: bool) -> Result<()> {
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

#[derive(Clone, Copy)]
struct LinuxExecutable {
    platform: &'static str,
}

fn validate_linux_executable(path: &Path) -> Result<LinuxExecutable> {
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
    validate_interpreter(path, &bytes)?;
    Ok(LinuxExecutable { platform })
}

fn validate_interpreter(path: &Path, bytes: &[u8]) -> Result<()> {
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
        if !interpreter.contains("ld-linux") {
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
