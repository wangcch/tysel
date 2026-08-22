use std::path::Path;

use tysel_distribution::{ChannelPointer, ReleaseManifest};

const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let kind = arguments.next().and_then(|value| value.into_string().ok()).ok_or_else(usage)?;
    let path = arguments.next().map(std::path::PathBuf::from).ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }
    let bytes = read_bounded(&path)?;
    match kind.as_str() {
        "release" => ReleaseManifest::from_json(&bytes)
            .map(|manifest| println!("valid release {}", manifest.version))
            .map_err(|error| error.to_string()),
        "channel" => ChannelPointer::from_json(&bytes)
            .map(|pointer| println!("valid channel {:?} {}", pointer.channel, pointer.version))
            .map_err(|error| error.to_string()),
        _ => Err(usage()),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        return Err("release metadata must be a regular file no larger than 4 MiB".into());
    }
    std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn usage() -> String {
    "usage: tysel-release-metadata <release|channel> <path>".into()
}
