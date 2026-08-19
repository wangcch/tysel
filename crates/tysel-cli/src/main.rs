//! Tysel CLI.
//!
//! Planned commands live in `roadmap.md` §21. M0 inspects manifests and can
//! append a TAP trailer onto a `tysel-service` stub (`tysel build`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tysel_manifest::Manifest;

#[derive(Parser)]
#[command(
    name = "tysel",
    version,
    about = "A lightweight native runtime for TypeScript services and agents.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Tysel application.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Watch, bundle, and run a service with reload.
    Dev { entry: Option<PathBuf> },
    /// Type-check, capability-scan, and validate the manifest.
    Check {
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Run an application without a production build.
    Run { entry: Option<PathBuf> },
    /// Run application tests.
    Test,
    /// Bundle the app and emit a single native executable.
    Build {
        entry: Option<PathBuf>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        release: bool,
        #[arg(long)]
        stub: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Print the effective capability and permission report.
    Inspect {
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Report npm/Web API compatibility for the current lockfile.
    Compat,
    /// Run the benchmark suite.
    Bench,
    /// Build a container image around the single executable.
    Image,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect { manifest } => inspect(manifest.as_path()),
        Commands::Build { entry, stub, output, manifest, target: _, profile: _, release: _ } => {
            build(manifest, entry, stub, output)
        }
        other => unimplemented_command(other),
    }
}

fn inspect(path: &Path) -> Result<()> {
    let manifest =
        Manifest::from_path(path).with_context(|| format!("failed to read {}", path.display()))?;
    print!("{}", manifest.inspect_report());
    Ok(())
}

fn build(
    manifest_path: PathBuf,
    entry: Option<PathBuf>,
    stub: Option<PathBuf>,
    output: Option<PathBuf>,
) -> Result<()> {
    let manifest = Manifest::from_path(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = entry.unwrap_or_else(|| root.join(&manifest.app.entry));
    let (bundle, source_map) = tysel_build::read_bundle(&entry)
        .with_context(|| format!("failed to read {}", entry.display()))?;
    let stub = resolve_stub(stub)?;
    let output = output.unwrap_or_else(|| PathBuf::from("dist").join(&manifest.app.name));
    let tap = tysel_build::tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), bundle, source_map);
    tysel_build::embed(&stub, &output, &tap)
        .with_context(|| format!("failed to write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn resolve_stub(stub: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = stub {
        return Ok(path);
    }
    if let Ok(path) = std::env::var("TYSEL_STUB") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    let candidate = exe.parent().unwrap_or(Path::new(".")).join("tysel-service");
    if candidate.is_file() {
        return Ok(candidate);
    }
    anyhow::bail!(
        "runtime stub not found; pass --stub or set TYSEL_STUB (looked for {})",
        candidate.display()
    )
}

fn unimplemented_command(command: Commands) -> Result<()> {
    let name = match command {
        Commands::Init { .. } => "init",
        Commands::Dev { .. } => "dev",
        Commands::Check { .. } => "check",
        Commands::Run { .. } => "run",
        Commands::Test => "test",
        Commands::Compat => "compat",
        Commands::Bench => "bench",
        Commands::Image => "image",
        Commands::Inspect { .. } | Commands::Build { .. } => unreachable!(),
    };
    anyhow::bail!("`tysel {name}` is not implemented yet; M0 spikes are next (see roadmap.md §26)")
}
