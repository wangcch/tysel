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
    }
    Ok(())
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}
