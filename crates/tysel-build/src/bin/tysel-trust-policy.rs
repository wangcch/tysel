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
    let current = arguments.next().map(PathBuf::from);
    let successor = arguments.next().map(PathBuf::from);
    let signature = arguments.next().map(PathBuf::from);
    let now = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .map(|value| value.parse::<u64>().context("invalid verification time"))
        .transpose()?;
    anyhow::ensure!(arguments.next().is_none(), usage());

    match (command.as_deref(), current, successor, signature, now) {
        (Some("validate"), Some(policy), None, None, None) => {
            validate_trust_policy(&read_policy(&policy)?)?;
            println!("valid trust policy {}", policy.display());
        }
        (Some("verify-transition"), Some(current), Some(successor), Some(signature), Some(now)) => {
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

fn read_policy(path: &Path) -> Result<TrustPolicy> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "trust policy is not a regular file");
    anyhow::ensure!(metadata.len() <= MAX_POLICY_BYTES, "trust policy is oversized");
    serde_json::from_slice(&std::fs::read(path)?)
        .with_context(|| format!("parse {}", path.display()))
}

fn usage() -> &'static str {
    "usage: tysel-trust-policy validate <policy> | verify-transition <current> <successor> <successor-signature> <now-unix>"
}
