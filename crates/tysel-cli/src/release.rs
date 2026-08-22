use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Subcommand;
use sha2::{Digest, Sha256};
use tysel_distribution::{BuildInfo, ReleaseManifest, Target};

use crate::platform;

#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Sign a verified release Evidence Index with an Ed25519 private key.
    Sign {
        artifact: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
    /// Verify all release evidence and its signature against a trust policy.
    Verify {
        artifact: PathBuf,
        #[arg(long)]
        trust: PathBuf,
    },
    /// Print the public key and derived key ID for a private release key.
    KeyInfo {
        #[arg(long)]
        key: PathBuf,
    },
    /// Compare two release archives and emit deterministic reproducibility evidence.
    Reproduce {
        first: PathBuf,
        second: PathBuf,
        #[arg(long)]
        source_commit: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        toolchain: String,
        #[arg(long, default_value = "Cargo.lock")]
        lockfile: PathBuf,
        #[arg(long = "command", required = true)]
        commands: Vec<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify one release archive against reproducibility evidence.
    VerifyReproducibility {
        artifact: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long, default_value = "Cargo.lock")]
        lockfile: PathBuf,
        #[arg(long)]
        target: String,
    },
    /// Sign a deterministic multi-architecture release archive.
    SignArtifact {
        artifact: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        key: PathBuf,
    },
    /// Verify a release archive signature against a trust policy.
    VerifyArtifact {
        artifact: PathBuf,
        #[arg(long)]
        trust: PathBuf,
        #[arg(long)]
        target: String,
    },
    /// Validate an extracted three-binary toolchain against a release manifest.
    #[command(hide = true)]
    VerifyInstallation {
        manifest: PathBuf,
        root: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        version: String,
    },
    /// Validate a release trust policy without performing network access.
    #[command(hide = true)]
    ValidateTrust { trust: PathBuf },
    /// Authenticate signed release metadata with an installed trust policy.
    #[command(hide = true)]
    VerifyMetadata {
        document: PathBuf,
        signature: PathBuf,
        #[arg(long)]
        trust: PathBuf,
    },
    /// Sign release metadata for hermetic release-pipeline fixtures.
    #[command(hide = true)]
    SignMetadata {
        document: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
}

pub fn run(command: ReleaseCommand) -> Result<()> {
    match command {
        ReleaseCommand::Sign { artifact, key } => {
            let signature = tysel_build::sign_release_evidence(&artifact, key, now_unix()?)?;
            println!("Signature        {}", signature.display());
        }
        ReleaseCommand::Verify { artifact, trust } => {
            let signature = tysel_build::verify_release_signature(&artifact, trust, now_unix()?)?;
            println!("Verified         {}", artifact.display());
            println!("Key ID           {}", signature.key_id);
        }
        ReleaseCommand::KeyInfo { key } => {
            let info = tysel_build::release_key_info(key)?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        ReleaseCommand::Reproduce {
            first,
            second,
            source_commit,
            target,
            toolchain,
            lockfile,
            commands,
            output,
        } => {
            let evidence = tysel_build::compare_reproducible_builds(
                first,
                second,
                &source_commit,
                &target,
                &toolchain,
                &commands,
                lockfile,
            )?;
            let output = tysel_build::write_reproducible_build_evidence(output, &evidence)?;
            println!("Reproducible      {}", evidence.artifact.sha256);
            println!("Evidence          {}", output.display());
        }
        ReleaseCommand::VerifyReproducibility { artifact, evidence, lockfile, target } => {
            let evidence = tysel_build::verify_reproducible_build_evidence(
                &artifact, evidence, lockfile, &target,
            )?;
            println!("Verified         {}", artifact.display());
            println!("Target           {}", evidence.target);
            println!("Commit           {}", evidence.source_commit);
        }
        ReleaseCommand::SignArtifact { artifact, target, key } => {
            let signature =
                tysel_build::sign_release_artifact(&artifact, &target, key, now_unix()?)?;
            println!("Signature        {}", signature.display());
        }
        ReleaseCommand::VerifyArtifact { artifact, trust, target } => {
            let signature = tysel_build::verify_release_artifact_signature(
                &artifact,
                trust,
                &target,
                now_unix()?,
            )?;
            println!("Verified         {}", artifact.display());
            println!("Target           {}", signature.target);
            println!("Key ID           {}", signature.key_id);
        }
        ReleaseCommand::VerifyInstallation { manifest, root, target, version } => {
            verify_installation(&manifest, &root, &target, &version)?;
            println!("Verified         {}", root.display());
            println!("Target           {target}");
            println!("Version          {version}");
        }
        ReleaseCommand::ValidateTrust { trust } => {
            let bytes = fs::read(&trust).with_context(|| format!("read {}", trust.display()))?;
            anyhow::ensure!(bytes.len() <= 1024 * 1024, "release trust policy is oversized");
            let policy: tysel_build::TrustPolicy =
                serde_json::from_slice(&bytes).context("parse release trust policy")?;
            tysel_build::validate_trust_policy(&policy)?;
            println!("Verified         {}", trust.display());
        }
        ReleaseCommand::VerifyMetadata { document, signature, trust } => {
            tysel_build::verify_release_metadata_signature(
                &document,
                &signature,
                &trust,
                now_unix()?,
            )?;
            println!("Verified         {}", document.display());
        }
        ReleaseCommand::SignMetadata { document, key } => {
            let signature = tysel_build::sign_release_metadata(&document, &key, now_unix()?)?;
            println!("Signature        {}", signature.display());
        }
    }
    Ok(())
}

pub(crate) fn verify_installation(
    manifest_path: &Path,
    root: &Path,
    target: &str,
    version: &str,
) -> Result<()> {
    let target = Target::from_canonical(target).context("unsupported canonical target")?;
    let manifest_bytes = fs::read(manifest_path)
        .with_context(|| format!("read release manifest {}", manifest_path.display()))?;
    anyhow::ensure!(manifest_bytes.len() <= 4 * 1024 * 1024, "release manifest is oversized");
    let manifest =
        ReleaseManifest::from_json(&manifest_bytes).context("validate release manifest")?;
    anyhow::ensure!(manifest.version == version, "release manifest version does not match request");
    let asset = manifest
        .assets
        .iter()
        .find(|asset| asset.target == target)
        .context("release manifest has no asset for requested target")?;
    platform::ensure_compatible(&asset.platform, target)?;

    for expected in &asset.files {
        let path = root.join(&expected.path);
        anyhow::ensure!(path.is_file(), "required release file is missing: {}", expected.path);
        anyhow::ensure!(
            hash_file(&path)? == expected.sha256,
            "release file hash mismatch: {}",
            expected.path
        );
    }

    let mut actual = Vec::new();
    for binary in ["tysel", "tysel-service", "tysel-worker"] {
        let path = root.join("bin").join(binary);
        let output = read_build_info(&path, binary)?;
        let info: BuildInfo = serde_json::from_slice(&output)
            .with_context(|| format!("parse {binary} build identity"))?;
        anyhow::ensure!(info.binary == binary, "{binary} reported the wrong binary identity");
        actual.push(info);
    }
    actual.sort_by(|left, right| left.binary.cmp(&right.binary));
    let mut expected = asset.build_info.clone();
    expected.sort_by(|left, right| left.binary.cmp(&right.binary));
    anyhow::ensure!(actual == expected, "extracted binaries do not match release identities");
    Ok(())
}

fn read_build_info(path: &Path, binary: &str) -> Result<Vec<u8>> {
    read_build_info_with_timeout(path, binary, Duration::from_secs(5))
}

fn read_build_info_with_timeout(path: &Path, binary: &str, timeout: Duration) -> Result<Vec<u8>> {
    let mut child = Command::new(path)
        .arg("--build-info-json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("query build identity for {binary}"))?;
    let stdout = child.stdout.take().context("capture build identity")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(64 * 1024 + 1).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            anyhow::bail!("{binary} build identity query timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = reader.join().map_err(|_| anyhow::anyhow!("build identity reader panicked"))??;
    anyhow::ensure!(status.success(), "{binary} rejected build identity query");
    anyhow::ensure!(output.len() <= 64 * 1024, "{binary} build identity is oversized");
    Ok(output)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn build_identity_query_has_a_hard_timeout() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir()
            .join(format!("tysel-release-hanging-build-info-{}", std::process::id()));
        fs::write(&path, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        let started = Instant::now();
        let error =
            read_build_info_with_timeout(&path, "tysel", Duration::from_millis(50)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        fs::remove_file(path).unwrap();
    }
}
