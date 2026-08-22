use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next().context(
        "usage: tysel-release-signer <artifact> (--target <target> | --metadata) --key <key>",
    )?;
    if first == "--key-info" {
        ensure!(
            args.next().as_deref() == Some(std::ffi::OsStr::new("--key")),
            "--key-info requires --key <key>"
        );
        let key = PathBuf::from(args.next().context("--key requires a value")?);
        ensure!(args.next().is_none(), "unexpected key-info argument");
        println!("{}", serde_json::to_string(&tysel_release_signing::release_key_info(key)?)?);
        return Ok(());
    }
    let artifact = PathBuf::from(first);
    let mut target = None;
    let mut metadata = false;
    let mut key = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--target") => {
                ensure!(target.is_none(), "--target may only be specified once");
                target = Some(required_utf8(args.next(), "--target requires a value")?);
            }
            Some("--key") => {
                ensure!(key.is_none(), "--key may only be specified once");
                key = Some(PathBuf::from(args.next().context("--key requires a value")?));
            }
            Some("--metadata") => {
                ensure!(!metadata, "--metadata may only be specified once");
                metadata = true;
            }
            _ => anyhow::bail!("unknown signer argument {}", argument.to_string_lossy()),
        }
    }
    ensure!(metadata != target.is_some(), "specify exactly one of --target or --metadata");
    let key = key.context("--key is required")?;
    let issued_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_secs();
    let signature = if metadata {
        tysel_release_signing::sign_release_metadata(artifact, key, issued_at_unix)?
    } else {
        tysel_release_signing::sign_release_artifact(
            artifact,
            target.as_deref().expect("validated target"),
            key,
            issued_at_unix,
        )?
    };
    println!("Signature        {}", signature.display());
    Ok(())
}

fn required_utf8(value: Option<OsString>, message: &str) -> Result<String> {
    value
        .with_context(|| message.to_owned())?
        .into_string()
        .map_err(|_| anyhow::anyhow!("{message}: value is not UTF-8"))
}
