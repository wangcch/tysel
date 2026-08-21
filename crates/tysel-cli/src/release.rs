use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Subcommand;

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
    }
    Ok(())
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}
