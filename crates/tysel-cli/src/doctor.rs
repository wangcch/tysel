use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tysel_distribution::{
    BuildInfo, Channel, ChannelPointer, InstallState, ManagedLayout, ReleaseManifest, Target,
};
use tysel_manifest::Manifest;

use crate::check::{self, Typecheck};
use crate::integrity::hash_file;
use crate::platform;

pub const DOCTOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Options {
    pub project: Option<PathBuf>,
    pub install_only: bool,
    pub network: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub passed: usize,
    pub warned: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub tool_version: &'static str,
    pub target: String,
    pub checks: Vec<Check>,
    pub summary: Summary,
}

impl Report {
    pub fn healthy(&self) -> bool {
        self.summary.failed == 0
    }
}

pub fn run(options: Options) -> Result<bool> {
    let report = collect(&options)?;
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    Ok(report.healthy())
}

fn collect(options: &Options) -> Result<Report> {
    let mut checks = Vec::new();
    let target = Target::current();
    collect_installation(&mut checks, target);
    collect_platform(&mut checks, target);
    if options.install_only {
        checks.push(skip("project.manifest", "project checks disabled by --install"));
    } else {
        collect_project(&mut checks, options.project.as_deref())?;
    }
    collect_network(&mut checks, options.network, target);
    Ok(report(checks, target))
}

fn report(checks: Vec<Check>, target: Target) -> Report {
    let summary = Summary {
        passed: checks.iter().filter(|check| check.status == Status::Pass).count(),
        warned: checks.iter().filter(|check| check.status == Status::Warn).count(),
        failed: checks.iter().filter(|check| check.status == Status::Fail).count(),
        skipped: checks.iter().filter(|check| check.status == Status::Skip).count(),
    };
    Report {
        schema_version: DOCTOR_SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION"),
        target: target.canonical().into(),
        checks,
        summary,
    }
}

fn collect_installation(checks: &mut Vec<Check>, target: Target) {
    let executable = match env::current_exe().and_then(fs::canonicalize) {
        Ok(path) => path,
        _ => {
            checks.push(fail(
                "install.executable",
                "cannot resolve the running tysel executable",
                "reinstall Tysel from a verified release",
            ));
            return;
        }
    };
    checks.push(pass("install.executable", "running executable resolved"));

    let managed = managed_root(&executable);
    match &managed {
        Some(_) => checks.push(pass("install.mode", "managed installation detected")),
        None => checks.push(warn(
            "install.mode",
            "unmanaged or source-build installation",
            "install with install.sh to enable managed upgrades",
        )),
    }

    let binary_dir = executable.parent().unwrap_or(Path::new("."));
    let expected = BuildInfo::current("tysel", env!("CARGO_PKG_VERSION"));
    let mut companions = Vec::new();
    let mut missing = false;
    for binary in ["tysel-service", "tysel-worker"] {
        let path = binary_dir.join(binary_name(binary));
        if !is_executable(&path) {
            missing = true;
            continue;
        }
        match read_build_info(&path) {
            Ok(info) if info.binary == binary => companions.push(info),
            _ => missing = true,
        }
    }
    if missing {
        checks.push(fail(
            "install.companions",
            "one or more companion binaries are missing, not executable, or unreadable",
            "reinstall the complete Tysel release instead of copying one binary",
        ));
        checks.push(skip("install.companion-version", "companion identity is unavailable"));
    } else {
        checks.push(pass("install.companions", "both companion binaries are executable"));
        if companions.iter().all(|info| expected.same_release_as(info)) {
            checks.push(pass(
                "install.companion-version",
                "all three binaries have one release identity",
            ));
        } else {
            checks.push(fail(
                "install.companion-version",
                "companion binaries come from mixed releases",
                "reinstall or upgrade the complete Tysel toolchain atomically",
            ));
        }
    }

    match &managed {
        Some(root) => {
            collect_managed_state(checks, root, target);
            match fs::metadata(root) {
                Ok(metadata) if !metadata.permissions().readonly() => {
                    checks.push(pass("install.upgrade-root", "managed root permits owner upgrades"))
                }
                _ => checks.push(fail(
                    "install.upgrade-root",
                    "managed root is unavailable or read-only",
                    "restore owner write permission before upgrading",
                )),
            }
        }
        None => {
            checks.push(skip("install.state", "unmanaged installations have no state.json"));
            checks.push(skip(
                "install.manifest-hashes",
                "unmanaged installation has no release manifest",
            ));
            checks.push(skip(
                "install.upgrade-root",
                "unmanaged installations cannot be upgraded by tysel",
            ));
        }
    }
    collect_path(checks, &executable, managed.is_some());
}

fn collect_managed_state(checks: &mut Vec<Check>, root: &Path, target: Target) {
    let layout = match ManagedLayout::new(root) {
        Ok(layout) => layout,
        Err(_) => {
            checks.push(fail("install.state", "managed root is invalid", "reinstall Tysel"));
            return;
        }
    };
    let state_bytes = match fs::read(layout.state_file()) {
        Ok(bytes) => bytes,
        Err(_) => {
            checks.push(fail(
                "install.state",
                "managed state.json is missing or unreadable",
                "reinstall Tysel or restore the previous state file",
            ));
            return;
        }
    };
    let state = match InstallState::from_json(&state_bytes) {
        Ok(state) if state.target == target => state,
        _ => {
            checks.push(fail(
                "install.state",
                "managed state is invalid or targets another platform",
                "reinstall the release for this platform",
            ));
            return;
        }
    };
    let active = match state.active_semver() {
        Ok(version) => version,
        Err(_) => unreachable!("validated install state"),
    };
    match fs::read_link(layout.active_bin_link()) {
        Ok(link) if link == layout.active_bin_target(&active) => {
            checks.push(pass("install.state", "state and active bin link agree"));
        }
        _ => {
            checks.push(fail(
                "install.state",
                "state and active bin link disagree",
                "run a managed upgrade or reinstall to restore the atomic link",
            ));
            return;
        }
    }

    let manifest_path = layout.version_manifest(&active);
    let manifest_bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            checks.push(fail(
                "install.manifest-hashes",
                "release manifest is missing or unreadable",
                "reinstall this version from a verified release",
            ));
            return;
        }
    };
    let manifest = match ReleaseManifest::from_json(&manifest_bytes) {
        Ok(manifest)
            if manifest.version == state.active_version
                && hex_sha256(&manifest_bytes) == state.manifest_sha256 =>
        {
            manifest
        }
        _ => {
            checks.push(fail(
                "install.manifest-hashes",
                "release manifest does not match managed state",
                "reinstall this version from a verified release",
            ));
            return;
        }
    };
    let Some(asset) = manifest.assets.iter().find(|asset| asset.target == target) else {
        checks.push(fail(
            "install.manifest-hashes",
            "release manifest has no asset for this platform",
            "install the release matching this platform",
        ));
        return;
    };
    collect_platform_requirements(checks, &asset.platform, target);
    let version_root = layout.version_dir(&active);
    let verified = asset.files.iter().all(|expected| {
        hash_file(&version_root.join(&expected.path)).is_ok_and(|hash| hash == expected.sha256)
    });
    if verified {
        checks.push(pass(
            "install.manifest-hashes",
            "managed binary hashes match the release manifest",
        ));
    } else {
        checks.push(fail(
            "install.manifest-hashes",
            "one or more managed binaries fail release hash verification",
            "reinstall this version from a verified release",
        ));
    }
}

fn collect_platform_requirements(
    checks: &mut Vec<Check>,
    requirements: &tysel_distribution::PlatformRequirements,
    target: Target,
) {
    match platform::evaluate(requirements, target) {
        Ok(requirement_checks) => {
            for requirement in requirement_checks {
                let id = match requirement.name {
                    "glibc" => "platform.minimum-glibc",
                    "Linux kernel" => "platform.minimum-kernel",
                    "macOS" => "platform.minimum-macos",
                    _ => "platform.minimum-version",
                };
                if requirement.compatible {
                    checks.push(pass(
                        id,
                        format!(
                            "{} {} satisfies minimum {}",
                            requirement.name, requirement.detected, requirement.required
                        ),
                    ));
                } else {
                    checks.push(fail(
                        id,
                        format!(
                            "{} {} is below required {}",
                            requirement.name, requirement.detected, requirement.required
                        ),
                        "upgrade the operating system before installing this Tysel release",
                    ));
                }
            }
        }
        Err(error) => checks.push(fail(
            "platform.minimum-version",
            format!("cannot evaluate release platform requirements: {error}"),
            "restore system version reporting or reinstall on a supported platform",
        )),
    }
}

fn collect_path(checks: &mut Vec<Check>, executable: &Path, managed: bool) {
    let Some(path) = env::var_os("PATH") else {
        checks.push(warn("install.path", "PATH is not set", "add the Tysel bin directory to PATH"));
        return;
    };
    let found = env::split_paths(&path)
        .map(|directory| directory.join(binary_name("tysel")))
        .find(|candidate| candidate.is_file());
    match found.and_then(|path| fs::canonicalize(path).ok()) {
        Some(path) if fs::canonicalize(executable).ok().as_deref() == Some(path.as_path()) => {
            checks.push(pass("install.path", "PATH resolves to the running Tysel release"));
        }
        Some(_) if managed => checks.push(fail(
            "install.path",
            "another tysel executable shadows the managed installation",
            "place the managed Tysel bin directory earlier in PATH",
        )),
        Some(_) => checks.push(warn(
            "install.path",
            "PATH resolves to a different tysel executable",
            "remove the stale entry or invoke the intended installation",
        )),
        None => checks.push(warn(
            "install.path",
            "tysel is not discoverable through PATH",
            "add the Tysel bin directory to PATH",
        )),
    }
}

fn collect_platform(checks: &mut Vec<Check>, target: Target) {
    if target == Target::Unsupported {
        checks.push(fail(
            "platform.target",
            "this operating system or architecture is unsupported",
            "use Linux or macOS on x64/arm64; Windows users should use WSL",
        ));
    } else {
        checks.push(pass("platform.target", format!("canonical target is {target}")));
    }
    match fs::metadata(env::temp_dir()) {
        Ok(metadata) if metadata.is_dir() && !metadata.permissions().readonly() => {
            checks.push(pass("platform.temp", "temporary directory is available"));
        }
        _ => checks.push(fail(
            "platform.temp",
            "temporary directory is unavailable or read-only",
            "set TMPDIR to a private writable directory",
        )),
    }
    #[cfg(target_os = "linux")]
    {
        let seccomp = fs::read_to_string("/proc/self/status").ok().and_then(|status| {
            status.lines().find(|line| line.starts_with("Seccomp:")).map(str::to_owned)
        });
        checks.push(match seccomp {
            Some(value) => pass("platform.linux-seccomp", value.trim().to_string()),
            None => warn(
                "platform.linux-seccomp",
                "seccomp status is unavailable",
                "ensure /proc is mounted for security diagnostics",
            ),
        });
        checks.push(if Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
            pass("platform.linux-cgroup", "cgroup v2 is available")
        } else {
            warn(
                "platform.linux-cgroup",
                "cgroup v2 is not visible",
                "enable cgroup v2 for production resource enforcement",
            )
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        checks.push(skip("platform.linux-seccomp", "Linux-only security probe"));
        checks.push(skip("platform.linux-cgroup", "Linux-only resource probe"));
    }
    #[cfg(target_os = "macos")]
    {
        let translated = Command::new("/usr/sbin/sysctl")
            .args(["-in", "sysctl.proc_translated"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| output.stdout.starts_with(b"1"));
        if translated {
            checks.push(warn(
                "platform.rosetta",
                "x64 Tysel is running through Rosetta",
                "install the darwin-arm64 release for native execution",
            ));
        } else {
            checks.push(pass("platform.rosetta", "process architecture is native"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    checks.push(skip("platform.rosetta", "macOS-only translation probe"));
}

fn collect_project(checks: &mut Vec<Check>, selected: Option<&Path>) -> Result<()> {
    let manifest_path = match selected {
        Some(path) if path.is_dir() => crate::project::discover_manifest(path)?,
        Some(path) => Some(path.to_path_buf()),
        None => crate::project::discover_manifest(
            &env::current_dir().context("resolve current directory")?,
        )?,
    };
    let Some(manifest_path) = manifest_path else {
        checks.push(skip("project.manifest", "no Tysel manifest found"));
        checks.push(skip("project.entry", "no project selected"));
        checks.push(skip("project.types-version", "no project selected"));
        checks.push(skip("project.typecheck", "no project selected"));
        return Ok(());
    };
    let manifest = match Manifest::from_path(&manifest_path) {
        Ok(manifest) => {
            checks.push(pass("project.manifest", "project manifest is valid"));
            manifest
        }
        Err(_) => {
            checks.push(fail(
                "project.manifest",
                "project manifest is invalid",
                "run `tysel check` for the validation error",
            ));
            checks.push(skip("project.entry", "manifest is invalid"));
            checks.push(skip("project.types-version", "manifest is invalid"));
            checks.push(skip("project.typecheck", "manifest is invalid"));
            return Ok(());
        }
    };
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = root.join(&manifest.app.entry);
    if entry.is_file() {
        let bundle_ok = entry.extension().and_then(|value| value.to_str()) == Some("wasm")
            || tysel_build::read_bundle(&entry).is_ok();
        checks.push(if bundle_ok {
            pass("project.entry", "entry and non-executing bundle scan passed")
        } else {
            fail(
                "project.entry",
                "entry exists but cannot be bundled safely",
                "run `tysel check` for import and syntax diagnostics",
            )
        });
    } else {
        checks.push(fail(
            "project.entry",
            "manifest entry does not exist",
            "correct app.entry or restore the source file",
        ));
    }
    collect_project_packages(checks, root);
    checks.push(match check::typecheck(root) {
        Typecheck::Ok => pass("project.typecheck", "TypeScript check passed"),
        Typecheck::Skipped(reason) => warn(
            "project.typecheck",
            format!("TypeScript check skipped: {reason}"),
            "install the pinned TypeScript development dependency",
        ),
        Typecheck::Failed(_) => fail(
            "project.typecheck",
            "TypeScript check failed",
            "run the project type-check command for compiler diagnostics",
        ),
    });
    Ok(())
}

fn collect_project_packages(checks: &mut Vec<Check>, root: &Path) {
    let package = fs::read(root.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let expected = env!("CARGO_PKG_VERSION");
    let version = |name: &str| {
        package.as_ref().and_then(|package| {
            package["dependencies"][name]
                .as_str()
                .or_else(|| package["devDependencies"][name].as_str())
        })
    };
    let types = version("@tysel/types");
    let test = version("@tysel/test");
    if types == Some(expected) && test == Some(expected) {
        checks
            .push(pass("project.types-version", "Tysel type packages match the native toolchain"));
    } else if types.is_none() && test.is_none() {
        checks.push(warn(
            "project.types-version",
            "Tysel type packages are not installed",
            format!("install @tysel/types@{expected} and @tysel/test@{expected}"),
        ));
    } else {
        checks.push(fail(
            "project.types-version",
            "Tysel type package versions do not match the native toolchain",
            format!("pin @tysel/types and @tysel/test to {expected}"),
        ));
    }
}

fn collect_network(checks: &mut Vec<Check>, enabled: bool, target: Target) {
    if !enabled {
        checks.push(skip("network.channel", "network checks require --network"));
        checks.push(skip("network.manifest", "network checks require --network"));
        checks.push(skip("network.asset", "network checks require --network"));
        return;
    }

    let base = env::var("TYSEL_DOWNLOAD_BASE")
        .unwrap_or_else(|_| "https://github.com/wangcch/tysel/releases".into());
    let pointer_url =
        format!("{}/latest/download/channel-pointer.json", base.trim_end_matches('/'));
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(format!("tysel-doctor/{}", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            checks.push(fail(
                "network.channel",
                "cannot initialize TLS/proxy release client",
                "check TLS certificate and proxy configuration",
            ));
            checks.push(skip("network.manifest", "channel check failed"));
            checks.push(skip("network.asset", "channel check failed"));
            return;
        }
    };
    let pointer_bytes = match get_bounded(&client, &pointer_url, 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) => {
            checks.push(fail(
                "network.channel",
                format!("stable channel is unreachable: {error}"),
                "check DNS, TLS, proxy settings, and TYSEL_DOWNLOAD_BASE",
            ));
            checks.push(skip("network.manifest", "channel check failed"));
            checks.push(skip("network.asset", "channel check failed"));
            return;
        }
    };
    let pointer = match ChannelPointer::from_json(&pointer_bytes) {
        Ok(pointer) if is_stable_channel(pointer.channel) => pointer,
        _ => {
            checks.push(fail(
                "network.channel",
                "stable channel metadata is invalid or unsupported",
                "use a supported immutable release or retry after metadata publication",
            ));
            checks.push(skip("network.manifest", "channel check failed"));
            checks.push(skip("network.asset", "channel check failed"));
            return;
        }
    };

    let trust = env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
        .and_then(|path| managed_root(&path))
        .map(|root| root.join("trust.json"));
    let signature_bytes = get_bounded(&client, &format!("{pointer_url}.sig.json"), 1024 * 1024);
    let authenticated = trust.as_deref().filter(|path| path.is_file()).and_then(|trust| {
        let signature_bytes = signature_bytes.ok()?;
        verify_metadata_bytes(&pointer_bytes, &signature_bytes, trust).ok()
    });
    if authenticated.is_some() {
        checks.push(pass("network.channel", format!("stable channel selects {}", pointer.version)));
    } else {
        checks.push(fail(
            "network.channel",
            "stable channel could not be authenticated with the installed trust policy",
            "reinstall from the official HTTPS bootstrap or restore trust.json",
        ));
        checks.push(skip("network.manifest", "channel authentication failed"));
        checks.push(skip("network.asset", "channel authentication failed"));
        return;
    }

    let manifest_bytes = match get_bounded(&client, &pointer.manifest_url, 4 * 1024 * 1024) {
        Ok(bytes)
            if bytes.len() as u64 == pointer.manifest_byte_size
                && hex_sha256(&bytes) == pointer.manifest_sha256 =>
        {
            bytes
        }
        _ => {
            checks.push(fail(
                "network.manifest",
                "immutable release manifest is unavailable or does not match the channel",
                "retry the release endpoint or select an immutable version",
            ));
            checks.push(skip("network.asset", "manifest check failed"));
            return;
        }
    };
    let manifest = match ReleaseManifest::from_json(&manifest_bytes) {
        Ok(manifest)
            if manifest.version == pointer.version
                && manifest.channel == Channel::Stable
                && manifest.channel == pointer.channel =>
        {
            manifest
        }
        _ => {
            checks.push(fail(
                "network.manifest",
                "immutable release manifest is invalid or version-mismatched",
                "use a supported release channel",
            ));
            checks.push(skip("network.asset", "manifest check failed"));
            return;
        }
    };
    let trust = trust.expect("authenticated channel has trust path");
    let manifest_signature = get_bounded(&client, &pointer.manifest_signature.url, 1024 * 1024);
    if manifest_signature
        .and_then(|signature| verify_metadata_bytes(&manifest_bytes, &signature, &trust))
        .is_err()
    {
        checks.push(fail(
            "network.manifest",
            "immutable release manifest signature is invalid",
            "do not install this release; retry the official release endpoint",
        ));
        checks.push(skip("network.asset", "manifest authentication failed"));
        return;
    }
    checks.push(pass("network.manifest", "immutable release manifest is authenticated"));

    let Some(asset) = manifest.assets.iter().find(|asset| asset.target == target) else {
        checks.push(fail(
            "network.asset",
            "stable release has no archive for this platform",
            "use a supported platform or immutable release",
        ));
        return;
    };
    let available =
        client.head(&asset.archive_url).send().is_ok_and(|response| response.status().is_success());
    checks.push(if available {
        pass("network.asset", format!("{} archive is reachable", target.canonical()))
    } else {
        fail(
            "network.asset",
            "target archive is not reachable",
            "check proxy access to the immutable release asset URL",
        )
    });
}

fn is_stable_channel(channel: Channel) -> bool {
    channel == Channel::Stable
}

fn get_bounded(client: &reqwest::blocking::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send()?.error_for_status()?;
    if response.content_length().is_some_and(|length| length > limit as u64) {
        anyhow::bail!("response exceeds {limit} bytes");
    }
    let mut bytes = Vec::new();
    response.by_ref().take(limit as u64 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= limit, "response exceeds {limit} bytes");
    Ok(bytes)
}

fn verify_metadata_bytes(document: &[u8], signature: &[u8], trust: &Path) -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root =
        env::temp_dir().join(format!("tysel-doctor-network-{}-{}", std::process::id(), nonce));
    fs::create_dir(&root)?;
    let document_path = root.join("document.json");
    let signature_path = root.join("document.json.sig.json");
    let result = (|| {
        fs::write(&document_path, document)?;
        fs::write(&signature_path, signature)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system clock predates Unix epoch")?
            .as_secs();
        tysel_build::verify_release_metadata_signature(
            &document_path,
            &signature_path,
            trust,
            now,
        )?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(root);
    result
}

fn managed_root(executable: &Path) -> Option<PathBuf> {
    let bin = executable.parent()?;
    let version = bin.parent()?;
    let versions = version.parent()?;
    if bin.file_name()? != "bin"
        || !version.file_name()?.to_str()?.starts_with('v')
        || versions.file_name()? != "versions"
    {
        return None;
    }
    versions.parent().map(Path::to_path_buf)
}

fn read_build_info(path: &Path) -> Result<BuildInfo> {
    let mut child = Command::new(path)
        .arg("--build-info-json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("query companion {}", path.display()))?;
    let stdout = child.stdout.take().context("capture companion build info")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(64 * 1024 + 1).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_millis(750);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            anyhow::bail!("companion build-info query timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output =
        reader.join().map_err(|_| anyhow::anyhow!("companion output reader panicked"))??;
    anyhow::ensure!(status.success(), "companion rejected build-info query");
    anyhow::ensure!(output.len() <= 64 * 1024, "companion build info is oversized");
    serde_json::from_slice(&output).context("parse companion build info")
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else { return false };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.into() }
}

fn pass(id: &'static str, summary: impl Into<String>) -> Check {
    Check { id, status: Status::Pass, summary: summary.into(), detail: None, remediation: None }
}

fn warn(id: &'static str, summary: impl Into<String>, remediation: impl Into<String>) -> Check {
    Check {
        id,
        status: Status::Warn,
        summary: summary.into(),
        detail: None,
        remediation: Some(remediation.into()),
    }
}

fn fail(id: &'static str, summary: impl Into<String>, remediation: impl Into<String>) -> Check {
    Check {
        id,
        status: Status::Fail,
        summary: summary.into(),
        detail: None,
        remediation: Some(remediation.into()),
    }
}

fn skip(id: &'static str, summary: impl Into<String>) -> Check {
    Check { id, status: Status::Skip, summary: summary.into(), detail: None, remediation: None }
}

fn print_human(report: &Report) {
    println!("Tysel doctor {} ({})", report.tool_version, report.target);
    for check in &report.checks {
        println!("{:4}  {:<28} {}", status_label(check.status), check.id, check.summary);
        if let Some(remediation) = &check.remediation {
            println!("      fix: {remediation}");
        }
    }
    println!(
        "Summary: {} passed, {} warned, {} failed, {} skipped",
        report.summary.passed, report.summary.warned, report.summary.failed, report.summary.skipped
    );
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Pass => "PASS",
        Status::Warn => "WARN",
        Status::Fail => "FAIL",
        Status::Skip => "SKIP",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_summary_and_json_are_stable_and_secret_free() {
        let report = report(
            vec![
                pass("install.mode", "managed"),
                warn("install.path", "shadowed", "fix PATH"),
                fail("install.companion-version", "mixed", "reinstall"),
                skip("network.channel", "disabled"),
            ],
            Target::LinuxX64,
        );
        assert_eq!(report.summary, Summary { passed: 1, warned: 1, failed: 1, skipped: 1 });
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"schemaVersion\": 1"));
        assert!(json.contains("install.companion-version"));
        assert!(!json.contains("authorization"));
        assert!(!report.healthy());
    }

    #[test]
    fn managed_root_requires_the_exact_version_layout() {
        assert_eq!(
            managed_root(Path::new("/opt/tysel/versions/v1.2.3/bin/tysel")),
            Some(PathBuf::from("/opt/tysel"))
        );
        assert!(managed_root(Path::new("/opt/tysel/bin/tysel")).is_none());
        assert!(managed_root(Path::new("/opt/tysel/versions/current/bin/tysel")).is_none());
    }

    #[test]
    fn nearest_manifest_walks_up_without_reading_project_source() {
        let root = env::temp_dir().join(format!("tysel-doctor-{}", std::process::id()));
        let nested = root.join("a/b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("tysel.toml"), "fixture").unwrap();
        assert_eq!(
            crate::project::discover_manifest(&nested).unwrap(),
            Some(root.join("tysel.toml"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn network_diagnostics_fail_closed_on_non_stable_channels() {
        assert!(is_stable_channel(Channel::Stable));
        assert!(!is_stable_channel(Channel::Beta));
        assert!(!is_stable_channel(Channel::Nightly));
    }

    #[cfg(unix)]
    #[test]
    fn companion_metadata_query_has_a_hard_timeout() {
        use std::os::unix::fs::PermissionsExt;

        let root = env::temp_dir().join(format!("tysel-doctor-timeout-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let companion = root.join("tysel-worker");
        fs::write(&companion, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        fs::set_permissions(&companion, fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        assert!(read_build_info(&companion).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        fs::remove_dir_all(root).unwrap();
    }
}
