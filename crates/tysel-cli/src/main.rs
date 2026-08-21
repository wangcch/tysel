//! Tysel CLI.
//!
//! `inspect` and `build` ship a TAP trailer. `check` validates a project.
//! `dev` watches sources and serves with process-level reload (roadmap §21).
//! `run` serves the same way without watching files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tysel_manifest::Manifest;

mod build;
mod check;
mod dev;
mod release;

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
    Dev {
        entry: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Type-check, capability-scan, and validate the manifest.
    Check {
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Run an application without watching files for reload.
    Run {
        entry: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Serve registered MCP tools over newline-delimited stdio.
    Mcp {
        entry: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
    /// Submit one message to a registered Queue handler and print its result.
    Queue {
        name: String,
        #[arg(long, default_value = "null")]
        input: String,
        #[arg(long)]
        message_id: Option<String>,
        entry: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
    },
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
    /// Sign or verify release evidence and inspect release keys.
    Release {
        #[command(subcommand)]
        command: release::ReleaseCommand,
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
        Commands::Check { manifest } => check::run(manifest.as_path()),
        Commands::Dev { entry, manifest } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run(manifest, entry))
        }
        Commands::Run { entry, manifest } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_once(manifest, entry))
        }
        Commands::Mcp { entry, manifest } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_mcp(manifest, entry))
        }
        Commands::Queue { name, input, message_id, entry, manifest } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_queue(manifest, entry, name, message_id, input))
        }
        Commands::Build { entry, stub, output, manifest, target, profile, release } => {
            build::run(manifest, entry, stub, output, target, profile, release)
        }
        Commands::Release { command } => release::run(command),
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
        Commands::Test => "test",
        Commands::Compat => "compat",
        Commands::Bench => "bench",
        Commands::Image => "image",
        Commands::Inspect { .. }
        | Commands::Build { .. }
        | Commands::Check { .. }
        | Commands::Dev { .. }
        | Commands::Run { .. }
        | Commands::Mcp { .. }
        | Commands::Release { .. }
        | Commands::Queue { .. } => unreachable!(),
    };
    anyhow::bail!("`tysel {name}` is not implemented yet (see roadmap.md §21)")
}
