use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tysel_build::{
    TrustPolicy, validate_trust_policy, validate_trust_policy_transition,
    verify_release_metadata_signature,
};

const MAX_POLICY_BYTES: u64 = 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments.next().and_then(|value| value.into_string().ok());
    match command.as_deref() {
        Some("validate") => {
            let policy = required_path(&mut arguments)?;
            anyhow::ensure!(arguments.next().is_none(), usage());
            validate_trust_policy(&read_policy(&policy)?)?;
            println!("valid trust policy {}", policy.display());
        }
        Some("verify") => {
            let policy = required_path(&mut arguments)?;
            let signature = required_path(&mut arguments)?;
            let now = required_time(&mut arguments)?;
            anyhow::ensure!(arguments.next().is_none(), usage());
            verify_release_metadata_signature(&policy, &signature, &policy, now)
                .context("authenticate trust policy")?;
            println!("valid trust-policy signature");
        }
        Some("verify-transition") => {
            let current = required_path(&mut arguments)?;
            let successor = required_path(&mut arguments)?;
            let signature = required_path(&mut arguments)?;
            let now = required_time(&mut arguments)?;
            anyhow::ensure!(arguments.next().is_none(), usage());
            let current_policy = read_policy(&current)?;
            let successor_policy = read_policy(&successor)?;
            validate_trust_policy_transition(&current_policy, &successor_policy)?;
            verify_release_metadata_signature(&successor, &signature, &current, now)
                .context("authenticate successor policy with current trust")?;
            verify_release_metadata_signature(&successor, &signature, &successor, now)
                .context("authenticate successor policy with successor trust")?;
            println!("valid trust-policy transition");
        }
        _ => anyhow::bail!(usage()),
    }
    Ok(())
}

fn required_path(arguments: &mut impl Iterator<Item = std::ffi::OsString>) -> Result<PathBuf> {
    arguments.next().map(PathBuf::from).context(usage())
}

fn required_time(arguments: &mut impl Iterator<Item = std::ffi::OsString>) -> Result<u64> {
    arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .context(usage())?
        .parse::<u64>()
        .context("invalid verification time")
}

fn read_policy(path: &Path) -> Result<TrustPolicy> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "trust policy is not a regular file");
    anyhow::ensure!(metadata.len() <= MAX_POLICY_BYTES, "trust policy is oversized");
    serde_json::from_slice(&std::fs::read(path)?)
        .with_context(|| format!("parse {}", path.display()))
}

fn usage() -> &'static str {
    "usage: tysel-trust-policy validate <policy> | verify <policy> <signature> <now-unix> | verify-transition <current> <successor> <successor-signature> <now-unix>"
}
