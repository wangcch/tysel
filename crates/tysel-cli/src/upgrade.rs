use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tysel_distribution::{
    Channel, ChannelPointer, INSTALL_STATE_SCHEMA_VERSION, InstallMethod, InstallState,
    ManagedLayout, ReleaseManifest, Target,
};

use crate::integrity::hash_file;
use crate::platform;
use crate::release;

const DEFAULT_DOWNLOAD_BASE: &str = "https://github.com/wangcch/tysel/releases";
const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    pub check: bool,
    pub version: Option<String>,
    pub channel: Option<String>,
    pub yes: bool,
    pub force: bool,
    pub rollback: bool,
    pub json: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpgradeReport {
    schema_version: u32,
    action: &'static str,
    changed: bool,
    from_version: String,
    to_version: String,
    target: String,
    summary: String,
}

pub fn run(options: Options) -> Result<()> {
    if let Some(channel) = options.channel.as_deref() {
        parse_channel(channel)?;
    }
    anyhow::ensure!(
        !(options.rollback
            && (options.check
                || options.version.is_some()
                || options.channel.is_some()
                || options.force)),
        "--rollback cannot be combined with --check, --version, --channel, or --force"
    );
    anyhow::ensure!(
        !(options.json && !options.check && !options.yes),
        "--json mutation requires --yes to avoid interactive prompting"
    );
    anyhow::ensure!(!(options.check && options.force), "--force has no effect with --check");
    let root = managed_root()?;
    let layout = ManagedLayout::new(&root)?;
    let _lock = UpgradeLock::acquire(layout.upgrade_lock(), Duration::from_secs(5))?;
    let state_bytes = fs::read(layout.state_file()).context("read managed state.json")?;
    let state = InstallState::from_json(&state_bytes)?;
    validate_active_install(&layout, &state)?;
    if options.rollback {
        return rollback(&layout, &state, &state_bytes, &options);
    }

    let trust = root.join("trust.json");
    validate_trust(&trust)?;
    let transaction = format!("upgrade-{}", std::process::id());
    let staging = layout.staging_dir(&transaction)?;
    if staging.exists() {
        fs::remove_dir_all(&staging).context("remove stale upgrade staging directory")?;
    }
    fs::create_dir_all(&staging).context("create upgrade staging directory")?;
    let result = upgrade(&layout, &state, &state_bytes, &trust, &staging, &options);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn upgrade(
    layout: &ManagedLayout,
    state: &InstallState,
    state_bytes: &[u8],
    trust: &Path,
    staging: &Path,
    options: &Options,
) -> Result<()> {
    if let Some(version) = options.version.as_deref() {
        Version::parse(version).context("invalid --version semantic version")?;
    }
    let client = release_client()?;
    let refreshed_trust = resolve_trust_policy(&client, trust, staging)?;
    let selected_channel =
        options.channel.as_deref().map(parse_channel).transpose()?.unwrap_or(state.channel);
    let (manifest, manifest_path) = resolve_manifest(
        &client,
        &refreshed_trust,
        staging,
        options.version.as_deref(),
        selected_channel,
    )?;
    let current = state.active_semver()?;
    let next = Version::parse(&manifest.version).context("parse selected release version")?;
    let updater = Version::parse(env!("CARGO_PKG_VERSION")).context("parse updater version")?;
    let minimum = Version::parse(&manifest.minimum_updater_version)
        .context("parse minimum updater version")?;
    anyhow::ensure!(updater >= minimum, "selected release requires tysel {minimum} or newer");
    let asset = manifest
        .assets
        .iter()
        .find(|asset| asset.target == state.target)
        .with_context(|| format!("selected release has no asset for {}", state.target))?;
    platform::ensure_compatible(&asset.platform, state.target)?;
    if next < current
        && downgrade_requires_force(
            state.channel,
            selected_channel,
            options.channel.is_some(),
            options.force,
        )
    {
        anyhow::bail!(
            "downgrading from {current} to {next} requires --force or an explicit --channel switch"
        );
    }
    if next == current && !options.force {
        let refreshed = fs::read(&refreshed_trust)?;
        let trust_changed = refreshed != fs::read(trust)?;
        if trust_changed && !options.check {
            confirm_trust_refresh(options)?;
            write_bytes_atomically(&layout.root().join("trust.json"), &refreshed)?;
            return emit(
                options,
                UpgradeReport {
                    schema_version: 1,
                    action: "trust-refresh",
                    changed: true,
                    from_version: current.to_string(),
                    to_version: next.to_string(),
                    target: state.target.canonical().into(),
                    summary: "refreshed the installed release trust policy".into(),
                },
            );
        }
        return emit(
            options,
            UpgradeReport {
                schema_version: 1,
                action: "check",
                changed: false,
                from_version: current.to_string(),
                to_version: next.to_string(),
                target: state.target.canonical().into(),
                summary: if trust_changed {
                    "Tysel is up to date; a trust-policy refresh is available".into()
                } else {
                    "Tysel is already up to date".into()
                },
            },
        );
    }
    if options.check {
        return emit(
            options,
            UpgradeReport {
                schema_version: 1,
                action: "check",
                changed: false,
                from_version: current.to_string(),
                to_version: next.to_string(),
                target: state.target.canonical().into(),
                summary: format!("upgrade available: {current} -> {next}"),
            },
        );
    }
    confirm(options, &current, &next)?;

    anyhow::ensure!(asset.byte_size as usize <= MAX_ARCHIVE_BYTES, "release archive is oversized");
    let archive = staging.join(format!("tysel-{}-{}.tar.gz", manifest.version, state.target));
    download_to(&client, &asset.archive_url, &archive, MAX_ARCHIVE_BYTES)?;
    anyhow::ensure!(fs::metadata(&archive)?.len() == asset.byte_size, "archive size mismatch");
    anyhow::ensure!(hash_file(&archive)? == asset.sha256, "archive SHA-256 mismatch");
    let signature = archive.with_file_name(format!(
        "{}.sig.json",
        archive.file_name().and_then(|value| value.to_str()).context("archive filename")?
    ));
    download_to(&client, &asset.signature.url, &signature, 1024 * 1024)?;
    tysel_build::verify_release_artifact_signature(
        &archive,
        &refreshed_trust,
        state.target.canonical(),
        now_unix()?,
    )?;

    let extracted = extract_archive(&archive, staging, &manifest.version, state.target)?;
    release::verify_installation(
        &manifest_path,
        &extracted,
        state.target.canonical(),
        &manifest.version,
    )?;
    fs::copy(&manifest_path, extracted.join("release-manifest.json"))?;
    let selected_manifest_bytes = fs::read(&manifest_path)?;
    let selected_manifest_sha = hash_file(&extracted.join("release-manifest.json"))?;

    let old_link = fs::read_link(layout.active_bin_link()).context("read active bin link")?;
    let previous_trust = fs::read(trust)?;
    let destination = layout.version_dir(&next);
    let destination_backup = staging.join("replaced-version");
    let mut replaced_destination = false;
    match fs::symlink_metadata(&destination) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "managed version destination is not a directory"
            );
            let existing_is_reusable = release::verify_installation(
                &manifest_path,
                &destination,
                state.target.canonical(),
                &manifest.version,
            )
            .is_ok()
                && installed_manifest_matches(&destination, &selected_manifest_bytes);
            if !existing_is_reusable {
                fs::rename(&destination, &destination_backup)
                    .context("quarantine damaged version directory")?;
                if let Err(error) = fs::rename(&extracted, &destination) {
                    let _ = fs::rename(&destination_backup, &destination);
                    return Err(error).context("publish repaired version directory");
                }
                replaced_destination = true;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(&extracted, &destination).context("publish verified version directory")?;
        }
        Err(error) => return Err(error).context("inspect managed version destination"),
    }

    let new_state = InstallState {
        schema_version: INSTALL_STATE_SCHEMA_VERSION,
        active_version: next.to_string(),
        previous_version: if next == current {
            state.previous_version.clone()
        } else {
            Some(current.to_string())
        },
        channel: manifest.channel,
        target: state.target,
        install_method: InstallMethod::Upgrade,
        manifest_sha256: selected_manifest_sha,
    };
    let activation = replace_link(layout, &next)
        .and_then(|()| write_state(layout, &new_state))
        .and_then(|()| {
            write_bytes_atomically(&layout.root().join("trust.json"), &fs::read(&refreshed_trust)?)
        })
        .and_then(|()| post_switch_doctor(layout));
    if let Err(error) = activation {
        let _ = replace_link_target(layout, &old_link);
        let _ = write_bytes_atomically(&layout.state_file(), state_bytes);
        let _ = write_bytes_atomically(&layout.root().join("trust.json"), &previous_trust);
        if replaced_destination {
            let _ = fs::remove_dir_all(&destination);
            let _ = fs::rename(&destination_backup, &destination);
        }
        return Err(error).context("upgrade activation failed; previous release was restored");
    }
    emit(
        options,
        UpgradeReport {
            schema_version: 1,
            action: "upgrade",
            changed: true,
            from_version: current.to_string(),
            to_version: next.to_string(),
            target: state.target.canonical().into(),
            summary: format!("upgraded Tysel from {current} to {next}"),
        },
    )
}

fn resolve_trust_policy(
    client: &reqwest::blocking::Client,
    current: &Path,
    staging: &Path,
) -> Result<PathBuf> {
    let base = env::var("TYSEL_DOWNLOAD_BASE").unwrap_or_else(|_| DEFAULT_DOWNLOAD_BASE.into());
    let release_base = format!("{}/download/trust", base.trim_end_matches('/'));
    let trust = staging.join("refreshed-trust.json");
    let signature = staging.join("refreshed-trust.json.sig.json");
    download_to(client, &format!("{release_base}/trust.json"), &trust, 1024 * 1024)?;
    download_to(client, &format!("{release_base}/trust.json.sig.json"), &signature, 1024 * 1024)?;
    let now = now_unix()?;
    tysel_build::verify_release_metadata_signature(&trust, &signature, current, now)
        .context("authenticate refreshed trust policy with installed trust")?;
    tysel_build::verify_release_metadata_signature(&trust, &signature, &trust, now)
        .context("validate refreshed trust policy and active signing key")?;
    let current_bytes = fs::read(current).context("read installed trust policy")?;
    let refreshed_bytes = fs::read(&trust).context("read refreshed trust policy")?;
    if refreshed_bytes != current_bytes {
        let current_policy: tysel_build::TrustPolicy =
            serde_json::from_slice(&current_bytes).context("parse installed trust policy")?;
        let refreshed_policy: tysel_build::TrustPolicy =
            serde_json::from_slice(&refreshed_bytes).context("parse refreshed trust policy")?;
        tysel_build::validate_trust_policy_transition(&current_policy, &refreshed_policy)
            .context("reject unsafe trust-policy transition")?;
    }
    Ok(trust)
}

fn rollback(
    layout: &ManagedLayout,
    state: &InstallState,
    state_bytes: &[u8],
    options: &Options,
) -> Result<()> {
    let previous = state.previous_version.as_deref().context("no retained previous version")?;
    let previous_version = Version::parse(previous).context("parse previous version")?;
    confirm(options, &state.active_semver()?, &previous_version)?;
    let manifest_path = layout.version_manifest(&previous_version);
    let previous_manifest = ReleaseManifest::from_json(&fs::read(&manifest_path)?)?;
    release::verify_installation(
        &manifest_path,
        &layout.version_dir(&previous_version),
        state.target.canonical(),
        previous,
    )?;
    let old_link = fs::read_link(layout.active_bin_link()).context("read active bin link")?;
    replace_link(layout, &previous_version)?;
    let rolled_back = InstallState {
        schema_version: INSTALL_STATE_SCHEMA_VERSION,
        active_version: previous.into(),
        previous_version: Some(state.active_version.clone()),
        channel: previous_manifest.channel,
        target: state.target,
        install_method: InstallMethod::Upgrade,
        manifest_sha256: hash_file(&manifest_path)?,
    };
    if let Err(error) = write_state(layout, &rolled_back).and_then(|()| post_switch_doctor(layout))
    {
        let _ = replace_link_target(layout, &old_link);
        let _ = write_bytes_atomically(&layout.state_file(), state_bytes);
        return Err(error).context("rollback activation failed; original release was restored");
    }
    emit(
        options,
        UpgradeReport {
            schema_version: 1,
            action: "rollback",
            changed: true,
            from_version: state.active_version.clone(),
            to_version: previous.into(),
            target: state.target.canonical().into(),
            summary: format!("rolled back Tysel to {previous}"),
        },
    )
}

fn resolve_manifest(
    client: &reqwest::blocking::Client,
    trust: &Path,
    staging: &Path,
    requested: Option<&str>,
    selected_channel: Channel,
) -> Result<(ReleaseManifest, PathBuf)> {
    let base = env::var("TYSEL_DOWNLOAD_BASE").unwrap_or_else(|_| DEFAULT_DOWNLOAD_BASE.into());
    let mut channel_contract = None;
    let manifest_url = if let Some(version) = requested {
        Version::parse(version).context("invalid --version semantic version")?;
        format!("{}/download/v{version}/release-manifest.json", base.trim_end_matches('/'))
    } else {
        let pointer_url = channel_pointer_url(&base, selected_channel);
        let pointer_path = staging.join("channel-pointer.json");
        let pointer_signature = staging.join("channel-pointer.json.sig.json");
        download_to(client, &pointer_url, &pointer_path, 1024 * 1024)?;
        download_to(client, &format!("{pointer_url}.sig.json"), &pointer_signature, 1024 * 1024)?;
        tysel_build::verify_release_metadata_signature(
            &pointer_path,
            &pointer_signature,
            trust,
            now_unix()?,
        )?;
        let pointer = ChannelPointer::from_json(&fs::read(&pointer_path)?)?;
        ensure_channel(pointer.channel, selected_channel, "channel pointer")?;
        let url = pointer.manifest_url.clone();
        channel_contract = Some(pointer);
        url
    };
    let manifest_path = staging.join("selected-release-manifest.json");
    let signature_path = staging.join("selected-release-manifest.json.sig.json");
    download_to(client, &manifest_url, &manifest_path, MAX_METADATA_BYTES)?;
    let signature_url = channel_contract.as_ref().map_or_else(
        || format!("{manifest_url}.sig.json"),
        |pointer| pointer.manifest_signature.url.clone(),
    );
    download_to(client, &signature_url, &signature_path, 1024 * 1024)?;
    if let Some(pointer) = &channel_contract {
        let bytes = fs::read(&manifest_path)?;
        anyhow::ensure!(
            bytes.len() as u64 == pointer.manifest_byte_size,
            "channel manifest size mismatch"
        );
        anyhow::ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == pointer.manifest_sha256,
            "channel manifest SHA-256 mismatch"
        );
    }
    let signature = tysel_build::verify_release_metadata_signature(
        &manifest_path,
        &signature_path,
        trust,
        now_unix()?,
    )?;
    let manifest = ReleaseManifest::from_json(&fs::read(&manifest_path)?)?;
    if requested.is_none() {
        ensure_channel(manifest.channel, selected_channel, "release manifest")?;
    }
    if let Some(pointer) = &channel_contract {
        anyhow::ensure!(manifest.channel == pointer.channel, "channel manifest channel mismatch");
        anyhow::ensure!(manifest.version == pointer.version, "channel manifest version mismatch");
        anyhow::ensure!(
            signature.key_id == pointer.manifest_signature.key_id,
            "channel selected an unexpected manifest signing key"
        );
    }
    if let Some(version) = requested {
        anyhow::ensure!(manifest.version == version, "immutable manifest version mismatch");
    }
    Ok((manifest, manifest_path))
}

fn release_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(format!("tysel-upgrade/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn download_to(
    client: &reqwest::blocking::Client,
    url: &str,
    destination: &Path,
    limit: usize,
) -> Result<()> {
    let mut response = client.get(url).send()?.error_for_status()?;
    if response.content_length().is_some_and(|length| length > limit as u64) {
        anyhow::bail!("download exceeds {limit} bytes");
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(destination)?;
    let copied = io::copy(&mut response.by_ref().take(limit as u64 + 1), &mut file)?;
    anyhow::ensure!(copied <= limit as u64, "download exceeds {limit} bytes");
    file.sync_all()?;
    Ok(())
}

fn extract_archive(
    archive: &Path,
    staging: &Path,
    version: &str,
    target: Target,
) -> Result<PathBuf> {
    let root_name = format!("tysel-{version}-{target}");
    let listing = Command::new("tar").args(["-tzf"]).arg(archive).output()?;
    anyhow::ensure!(listing.status.success(), "cannot list release archive");
    let listing =
        String::from_utf8(listing.stdout).context("archive member names are not UTF-8")?;
    for member in listing.lines() {
        anyhow::ensure!(
            !member.starts_with('/')
                && !member.contains("//")
                && member
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .all(|component| component != "." && component != ".."),
            "archive contains unsafe member {member}"
        );
        let allowed = matches!(
            member,
            value if value == root_name
                || value == format!("{root_name}/")
                || value == format!("{root_name}/bin")
                || value == format!("{root_name}/bin/")
                || value == format!("{root_name}/bin/tysel")
                || value == format!("{root_name}/bin/tysel-service")
                || value == format!("{root_name}/bin/tysel-worker")
                || value == format!("{root_name}/LICENSE")
                || value == format!("{root_name}/README.md")
                || value == format!("{root_name}/share")
                || value == format!("{root_name}/share/")
                || value == format!("{root_name}/share/acceptance")
                || value == format!("{root_name}/share/acceptance/")
                || value.starts_with(&format!("{root_name}/share/acceptance/"))
        );
        anyhow::ensure!(allowed, "archive contains unexpected member {member}");
    }
    let details = Command::new("tar").args(["-tvzf"]).arg(archive).output()?;
    anyhow::ensure!(details.status.success(), "cannot inspect release archive");
    for line in String::from_utf8(details.stdout)?.lines() {
        anyhow::ensure!(
            matches!(line.as_bytes().first(), Some(b'-' | b'd')),
            "archive contains a link or device"
        );
    }
    let extract = staging.join("extract");
    fs::create_dir(&extract)?;
    let status = Command::new("sh")
        // POSIX shells may use 512-byte blocks for -f, while bash commonly
        // uses KiB. This permits at least 512 MiB per file; tree_size below
        // enforces the exact 512 MiB aggregate limit.
        .args(["-c", "ulimit -f 1048576; exec tar -xzf \"$1\" -C \"$2\"", "sh"])
        .arg(archive)
        .arg(&extract)
        .status()?;
    anyhow::ensure!(status.success(), "cannot extract release archive");
    let extracted = extract.join(root_name);
    anyhow::ensure!(tree_size(&extracted)? <= 512 * 1024 * 1024, "extracted release is oversized");
    Ok(extracted)
}

fn tree_size(path: &Path) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(!metadata.file_type().is_symlink(), "extracted release contains a link");
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    anyhow::ensure!(metadata.is_dir(), "extracted release contains a special file");
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total
            .checked_add(tree_size(&entry?.path())?)
            .context("extracted release size overflow")?;
    }
    Ok(total)
}

fn installed_manifest_matches(destination: &Path, selected: &[u8]) -> bool {
    fs::read(destination.join("release-manifest.json")).is_ok_and(|installed| installed == selected)
}

fn validate_active_install(layout: &ManagedLayout, state: &InstallState) -> Result<()> {
    anyhow::ensure!(state.target == Target::current(), "managed state targets another platform");
    let active = state.active_semver()?;
    anyhow::ensure!(
        fs::read_link(layout.active_bin_link())? == layout.active_bin_target(&active),
        "managed state and active bin link disagree"
    );
    release::verify_installation(
        &layout.version_manifest(&active),
        &layout.version_dir(&active),
        state.target.canonical(),
        &state.active_version,
    )
}

fn validate_trust(path: &Path) -> Result<()> {
    let bytes = fs::read(path).context("managed trust.json is missing")?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "managed trust policy is oversized");
    let policy: tysel_build::TrustPolicy = serde_json::from_slice(&bytes)?;
    tysel_build::validate_trust_policy(&policy)
}

fn managed_root() -> Result<PathBuf> {
    let executable =
        fs::canonicalize(env::current_exe().context("resolve running tysel executable")?)
            .context("canonicalize running tysel executable")?;
    let bin = executable.parent().context("running executable has no bin directory")?;
    let version = bin.parent().context("running executable has no version directory")?;
    let versions = version.parent().context("running executable has no versions directory")?;
    anyhow::ensure!(
        bin.file_name().and_then(|value| value.to_str()) == Some("bin")
            && version
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with('v'))
            && versions.file_name().and_then(|value| value.to_str()) == Some("versions"),
        "tysel upgrade only supports installations managed by install.sh"
    );
    Ok(versions.parent().context("managed installation has no root")?.to_path_buf())
}

fn confirm(options: &Options, from: &Version, to: &Version) -> Result<()> {
    if options.yes {
        return Ok(());
    }
    anyhow::ensure!(io::stdin().is_terminal(), "non-interactive upgrade requires --yes");
    eprint!("Switch Tysel from {from} to {to}? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    anyhow::ensure!(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"), "upgrade cancelled");
    Ok(())
}

fn confirm_trust_refresh(options: &Options) -> Result<()> {
    if options.yes {
        return Ok(());
    }
    anyhow::ensure!(io::stdin().is_terminal(), "non-interactive trust refresh requires --yes");
    eprint!("Refresh the installed Tysel release trust policy? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    anyhow::ensure!(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"), "trust refresh cancelled");
    Ok(())
}

fn parse_channel(value: &str) -> Result<Channel> {
    match value {
        "stable" => Ok(Channel::Stable),
        "canary" => Ok(Channel::Canary),
        _ => anyhow::bail!("unsupported release channel {value}; expected stable or canary"),
    }
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "stable",
        Channel::Canary => "canary",
    }
}

fn channel_pointer_url(base: &str, channel: Channel) -> String {
    let base = base.trim_end_matches('/');
    match channel {
        Channel::Stable => format!("{base}/latest/download/channel-pointer.json"),
        Channel::Canary => format!("{base}/download/canary/channel-pointer.json"),
    }
}

fn ensure_channel(actual: Channel, expected: Channel, document: &str) -> Result<()> {
    anyhow::ensure!(
        actual == expected,
        "{document} is for {}, expected {}",
        channel_name(actual),
        channel_name(expected)
    );
    Ok(())
}

fn downgrade_requires_force(
    installed_channel: Channel,
    selected_channel: Channel,
    channel_was_explicit: bool,
    force: bool,
) -> bool {
    !force && !(channel_was_explicit && selected_channel != installed_channel)
}

fn replace_link(layout: &ManagedLayout, version: &Version) -> Result<()> {
    replace_link_target(layout, &layout.active_bin_target(version))
}

#[cfg(unix)]
fn replace_link_target(layout: &ManagedLayout, target: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;
    let temporary = layout.root().join(format!(".bin-new-{}", std::process::id()));
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    symlink(target, &temporary)?;
    fs::rename(&temporary, layout.active_bin_link())?;
    Ok(())
}

#[cfg(not(unix))]
fn replace_link_target(_layout: &ManagedLayout, _target: &Path) -> Result<()> {
    anyhow::bail!("managed upgrades are supported on Unix platforms only")
}

fn write_state(layout: &ManagedLayout, state: &InstallState) -> Result<()> {
    state.validate()?;
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    write_bytes_atomically(&layout.state_file(), &bytes)
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().and_then(|value| value.to_str()).context("managed file name")?;
    let temporary = path.with_file_name(format!(".{name}-new-{}", std::process::id()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn post_switch_doctor(layout: &ManagedLayout) -> Result<()> {
    let binary = layout.active_bin_link().join("tysel");
    let diagnostics_path =
        layout.root().join(format!(".post-switch-doctor-{}", std::process::id()));
    let diagnostics = OpenOptions::new().write(true).create_new(true).open(&diagnostics_path)?;
    let path = env::var_os("PATH").unwrap_or_default();
    let joined =
        env::join_paths(std::iter::once(layout.active_bin_link()).chain(env::split_paths(&path)))?;
    let status = (|| -> Result<_> {
        let mut child = Command::new(binary)
            .args(["doctor", "--install", "--json"])
            .env("PATH", joined)
            .stdin(Stdio::null())
            .stdout(Stdio::from(diagnostics.try_clone()?))
            .stderr(Stdio::from(diagnostics))
            .spawn()?;
        wait_for_child(&mut child, Duration::from_secs(15), "post-switch doctor")
    })();
    let output = fs::read_to_string(&diagnostics_path).unwrap_or_default();
    let _ = fs::remove_file(&diagnostics_path);
    let status = status?;
    anyhow::ensure!(
        status.success(),
        "post-switch doctor failed{}",
        if output.trim().is_empty() { String::new() } else { format!(":\n{}", output.trim()) }
    );
    Ok(())
}

fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
    description: &str,
) -> Result<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("{description} timed out after {} seconds", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn emit(options: &Options, report: UpgradeReport) -> Result<()> {
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", report.summary);
    }
    Ok(())
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

struct UpgradeLock(PathBuf);

impl UpgradeLock {
    fn acquire(path: PathBuf, timeout: Duration) -> Result<Self> {
        let candidate =
            path.with_file_name(format!(".upgrade-lock-{}-{}", std::process::id(), now_unix()?));
        let mut candidate_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .context("create upgrade lock candidate")?;
        writeln!(candidate_file, "{}", std::process::id())?;
        candidate_file.sync_all()?;
        let started = std::time::Instant::now();
        loop {
            match fs::hard_link(&candidate, &path) {
                Ok(()) => {
                    fs::remove_file(&candidate)?;
                    return Ok(Self(path));
                }
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        && started.elapsed() < timeout =>
                {
                    if stale_lock(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&candidate);
                    anyhow::bail!("another upgrade holds {}", path.display())
                }
                Err(error) => {
                    let _ = fs::remove_file(&candidate);
                    return Err(error).context("acquire upgrade lock");
                }
            }
        }
    }
}

fn stale_lock(path: &Path) -> bool {
    let Ok(pid) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = pid.trim().parse::<i32>() else {
        return false;
    };
    !process_alive(pid)
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 performs a liveness/permission check and never signals the process.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().kind() == io::ErrorKind::PermissionDenied
}

#[cfg(not(unix))]
fn process_alive(_pid: i32) -> bool {
    true
}

impl Drop for UpgradeLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "tysel-upgrade-{label}-{}-{}",
            std::process::id(),
            now_unix().unwrap()
        ))
    }

    #[cfg(unix)]
    #[test]
    fn activation_replaces_one_relative_link_atomically() {
        let root = root("activate");
        fs::create_dir_all(root.join("versions/v1.0.0/bin")).unwrap();
        fs::create_dir_all(root.join("versions/v1.1.0/bin")).unwrap();
        let layout = ManagedLayout::new(&root).unwrap();
        std::os::unix::fs::symlink("versions/v1.0.0/bin", layout.active_bin_link()).unwrap();
        replace_link(&layout, &Version::parse("1.1.0").unwrap()).unwrap();
        assert_eq!(
            fs::read_link(layout.active_bin_link()).unwrap(),
            Path::new("versions/v1.1.0/bin")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_upgrade_lock_fails_closed() {
        let root = root("lock");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("upgrade.lock");
        let first = UpgradeLock::acquire(path.clone(), Duration::ZERO).unwrap();
        assert!(UpgradeLock::acquire(path.clone(), Duration::ZERO).is_err());
        drop(first);
        UpgradeLock::acquire(path, Duration::ZERO).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_upgrade_lock_is_reclaimed() {
        let root = root("stale-lock");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("upgrade.lock");
        fs::write(&path, "2147483647\n").unwrap();
        let lock = UpgradeLock::acquire(path.clone(), Duration::from_millis(250)).unwrap();
        assert!(path.is_file());
        drop(lock);
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn channel_resolution_requires_an_exact_match() {
        ensure_channel(Channel::Stable, Channel::Stable, "channel pointer").unwrap();
        ensure_channel(Channel::Canary, Channel::Canary, "release manifest").unwrap();
        let error =
            ensure_channel(Channel::Canary, Channel::Stable, "channel pointer").unwrap_err();
        assert!(error.to_string().contains("for canary, expected stable"));
    }

    #[test]
    fn channel_endpoints_are_physically_separate() {
        let base = "https://github.com/wangcch/tysel/releases/";
        assert_eq!(
            channel_pointer_url(base, Channel::Stable),
            "https://github.com/wangcch/tysel/releases/latest/download/channel-pointer.json"
        );
        assert_eq!(
            channel_pointer_url(base, Channel::Canary),
            "https://github.com/wangcch/tysel/releases/download/canary/channel-pointer.json"
        );
    }

    #[test]
    fn only_a_real_channel_switch_can_bypass_the_downgrade_guard() {
        assert!(downgrade_requires_force(Channel::Stable, Channel::Stable, true, false));
        assert!(downgrade_requires_force(Channel::Canary, Channel::Canary, true, false));
        assert!(!downgrade_requires_force(Channel::Canary, Channel::Stable, true, false));
        assert!(!downgrade_requires_force(Channel::Stable, Channel::Stable, false, true));
    }

    #[test]
    fn version_directory_is_reused_only_for_the_authenticated_manifest_bytes() {
        let root = root("manifest-reuse");
        fs::create_dir_all(&root).unwrap();
        let selected = br#"{"schemaVersion":1}"#;
        fs::write(root.join("release-manifest.json"), selected).unwrap();
        assert!(installed_manifest_matches(&root, selected));

        fs::write(root.join("release-manifest.json"), [selected.as_slice(), b"\n"].concat())
            .unwrap();
        assert!(!installed_manifest_matches(&root, selected));
        fs::remove_file(root.join("release-manifest.json")).unwrap();
        assert!(!installed_manifest_matches(&root, selected));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn child_deadline_kills_a_hung_post_switch_check() {
        let mut child = Command::new("sh")
            .args(["-c", "while :; do :; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let error =
            wait_for_child(&mut child, Duration::from_millis(50), "test child").unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
