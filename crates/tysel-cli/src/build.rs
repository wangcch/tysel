use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tysel_manifest::Manifest;

use crate::check::{Typecheck, typecheck};

pub fn run(
    manifest_path: PathBuf,
    entry: Option<PathBuf>,
    stub: Option<PathBuf>,
    output: Option<PathBuf>,
    target: Option<String>,
    profile: Option<String>,
    release: bool,
) -> Result<()> {
    let manifest = Manifest::from_path(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let host = host_target();
    if let Some(requested) = target.as_deref() {
        if !target_allowed(requested, host.aliases) {
            return Err(anyhow!(
                "cross-compilation is not implemented; this host is {} (omit --target)",
                host.label
            ));
        }
    }
    if let Some(requested) = profile.as_deref() {
        if requested != manifest.app.profile {
            return Err(anyhow!(
                "--profile {requested} does not match manifest profile {}",
                manifest.app.profile
            ));
        }
    }
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = entry.unwrap_or_else(|| root.join(&manifest.app.entry));
    if !entry.is_file() {
        return Err(anyhow!("entry not found: {}", entry.display()));
    }
    let (bundle, source_map) = tysel_build::read_bundle(&entry)
        .with_context(|| format!("failed to read {}", entry.display()))?;
    let types = typecheck(root);
    let type_line = match &types {
        Typecheck::Ok => "passed".to_owned(),
        Typecheck::Skipped(reason) => format!("skipped ({reason})"),
        Typecheck::Failed(_) => "failed".to_owned(),
    };
    if let Typecheck::Failed(output) = types {
        eprint!("{output}");
        return Err(anyhow!("TypeScript check failed"));
    }
    let stub = resolve_stub(stub, release)?;
    let output = output.unwrap_or_else(|| PathBuf::from("dist").join(&manifest.app.name));
    let tap = tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), bundle, source_map);
    tysel_build::embed(&stub, &output, &tap)
        .with_context(|| format!("failed to write {}", output.display()))?;
    let executable = std::fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
    println!("Type check       {type_line}");
    println!("Bundle           {}", format_bytes(tap.bundle.len() as u64));
    println!("Capabilities     {}", capability_summary(&manifest));
    println!("Runtime          {}", manifest.app.profile);
    println!("Executable       {}", format_bytes(executable));
    println!("Target           {}", host.label);
    println!("Output           {}", output.display());
    Ok(())
}

struct HostTarget {
    label: &'static str,
    aliases: &'static [&'static str],
}

fn host_target() -> HostTarget {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        HostTarget {
            label: "darwin-arm64",
            aliases: &["darwin-arm64", "macos-arm64", "aarch64-apple-darwin"],
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        HostTarget {
            label: "darwin-x64",
            aliases: &["darwin-x64", "macos-x64", "x86_64-apple-darwin"],
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        HostTarget {
            label: "linux-x64",
            aliases: &["linux-x64", "linux-amd64", "x86_64-unknown-linux-gnu"],
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        HostTarget { label: "linux-arm64", aliases: &["linux-arm64", "aarch64-unknown-linux-gnu"] }
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        HostTarget { label: "unknown", aliases: &[] }
    }
}

fn target_allowed(requested: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| alias.eq_ignore_ascii_case(requested))
}

fn capability_summary(manifest: &Manifest) -> String {
    let mut caps = Vec::new();
    if !manifest.permissions.fetch.is_empty() {
        caps.push("http");
    }
    if !manifest.permissions.secrets.is_empty() {
        caps.push("secrets");
    }
    if manifest.durable.store == "sqlite" {
        caps.push("sqlite");
    }
    if !manifest.permissions.postgres.is_empty() {
        caps.push("postgres");
    }
    if !manifest.permissions.fs_read.is_empty() || !manifest.permissions.fs_write.is_empty() {
        caps.push("fs");
    }
    if caps.is_empty() { "none".into() } else { caps.join(", ") }
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if n >= MB {
        let mb = n as f64 / MB as f64;
        format!("{:.1} MB", (mb * 10.0).round() / 10.0)
    } else if n >= KB {
        format!("{} KB", n.div_ceil(KB))
    } else {
        format!("{n} B")
    }
}

fn resolve_stub(stub: Option<PathBuf>, release: bool) -> Result<PathBuf> {
    if let Some(path) = stub {
        if !path.is_file() {
            return Err(anyhow!("runtime stub not found at {}", path.display()));
        }
        return Ok(path);
    }
    if let Ok(path) = std::env::var("TYSEL_STUB") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(anyhow!("runtime stub not found at {}", path.display()));
    }
    for candidate in stub_candidates(release) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "runtime stub not found; pass --stub, set TYSEL_STUB, or build with `cargo build -p tysel-runtime --bin tysel-service{}`",
        if release { " --release" } else { "" }
    )
}

fn stub_candidates(release: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let sibling = std::env::current_exe()
        .ok()
        .map(|exe| with_exe(exe.parent().unwrap_or(Path::new(".")).join("tysel-service")));
    if !release {
        if let Some(path) = sibling.clone() {
            out.push(path);
        }
    }
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        push_cargo_stubs(&mut out, Path::new(&dir), release);
    }
    if let Ok(mut dir) = std::env::current_dir() {
        loop {
            push_cargo_stubs(&mut out, &dir.join("target"), release);
            if !dir.pop() {
                break;
            }
        }
    }
    if release {
        if let Some(path) = sibling {
            out.push(path);
        }
    }
    if let Some(path) = find_on_path("tysel-service") {
        out.push(path);
    }
    out
}

fn push_cargo_stubs(out: &mut Vec<PathBuf>, target: &Path, release: bool) {
    if release {
        out.push(with_exe(target.join("release/tysel-service")));
        return;
    }
    out.push(with_exe(target.join("debug/tysel-service")));
    out.push(with_exe(target.join("release/tysel-service")));
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = with_exe(dir.join(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn with_exe(mut path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_target_accepts_its_aliases() {
        let host = host_target();
        assert!(target_allowed(host.label, host.aliases));
        for alias in host.aliases {
            assert!(target_allowed(alias, host.aliases), "{alias}");
        }
        assert!(!target_allowed("linux-riscv64", host.aliases));
    }

    #[test]
    fn format_bytes_uses_kb_and_mb() {
        assert_eq!(format_bytes(200), "200 B");
        assert_eq!(format_bytes(184 * 1024), "184 KB");
        assert_eq!(format_bytes(14 * 1024 * 1024), "14.0 MB");
    }
}
