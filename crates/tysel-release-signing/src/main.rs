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
    let artifact = PathBuf::from(
        args.next()
            .context("usage: tysel-release-signer <artifact> --target <target> --key <key>")?,
    );
    let mut target = None;
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
            _ => anyhow::bail!("unknown signer argument {}", argument.to_string_lossy()),
        }
    }
    let target = target.context("--target is required")?;
    let key = key.context("--key is required")?;
    let issued_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_secs();
    let signature =
        tysel_release_signing::sign_release_artifact(artifact, &target, key, issued_at_unix)?;
    println!("Signature        {}", signature.display());
    Ok(())
}

fn required_utf8(value: Option<OsString>, message: &str) -> Result<String> {
    value
        .with_context(|| message.to_owned())?
        .into_string()
        .map_err(|_| anyhow::anyhow!("{message}: value is not UTF-8"))
}
