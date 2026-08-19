//! Tysel CLI.
//!
//! Planned commands live in `roadmap.md` §21. M0 only guarantees that the
//! binary builds, prints version/help, and can inspect a `tysel.toml`.

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
        other => unimplemented_command(other),
    }
}

fn inspect(path: &Path) -> Result<()> {
    let manifest =
        Manifest::from_path(path).with_context(|| format!("failed to read {}", path.display()))?;
    print!("{}", manifest.inspect_report());
    Ok(())
}

fn unimplemented_command(command: Commands) -> Result<()> {
    let name = match command {
        Commands::Init { .. } => "init",
        Commands::Dev { .. } => "dev",
        Commands::Check { .. } => "check",
        Commands::Run { .. } => "run",
        Commands::Test => "test",
        Commands::Build { .. } => "build",
        Commands::Compat => "compat",
        Commands::Bench => "bench",
        Commands::Image => "image",
        Commands::Inspect { .. } => unreachable!(),
    };
    anyhow::bail!("`tysel {name}` is not implemented yet; M0 spikes are next (see roadmap.md §26)")
}
