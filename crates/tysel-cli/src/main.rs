//! Tysel CLI.
//!
//! `inspect` and `build` ship a TAP trailer. `check` validates a project.
//! `dev` watches sources and serves with process-level reload (roadmap §21).
//! `run` serves the same way without watching files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use tysel_manifest::Manifest;

mod bench;
mod build;
mod check;
mod compat;
mod dev;
mod image;
mod init;
mod node_scan;
mod release;
mod test_runner;

#[derive(Parser)]
#[command(
    name = "tysel",
    version,
    about = "A lightweight native runtime for TypeScript services and agents.",
    arg_required_else_help = true
)]
struct Cli {
    /// Format fatal CLI errors for humans or automation.
    #[arg(long, global = true, value_enum, default_value_t = ErrorFormat::Human)]
    error_format: ErrorFormat,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, ValueEnum)]
enum ErrorFormat {
    Human,
    Json,
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
    /// Run application tests in isolated QuickJS instances.
    Test {
        /// Test files or directories (defaults to tests/).
        paths: Vec<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
        /// Timeout for each test.
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        /// Emit a stable machine-readable report.
        #[arg(long)]
        json: bool,
    },
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
    /// Report npm/Web API compatibility for the current project.
    Compat {
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
        /// Emit a stable machine-readable report.
        #[arg(long)]
        json: bool,
        /// Exit unsuccessfully when unsupported packages are found.
        #[arg(long)]
        strict: bool,
        /// With --strict, also reject packages not in the catalog.
        #[arg(long, requires = "strict")]
        deny_unknown: bool,
    },
    /// Run the roadmap §23 benchmark harness.
    Bench {
        /// Suite to run: startup, memory, isolate, task, durable, http, or all.
        suite: bench::BenchSuite,
        /// Human table or stable JSON.
        #[arg(long, value_enum, default_value_t = bench::BenchFormat::Human)]
        format: bench::BenchFormat,
        /// Write complete benchmark evidence with raw samples (requires release `all`, full scale).
        #[arg(long)]
        evidence: Option<PathBuf>,
        /// Source commit recorded in evidence (defaults to HEAD in a clean source workspace).
        #[arg(long, requires = "evidence")]
        source_commit: Option<String>,
        /// Command line recorded in evidence (defaults to the current invocation).
        #[arg(long, requires = "evidence")]
        command: Option<String>,
        /// Deprecated compatibility flag; every benchmark suite now has a harness.
        #[arg(long)]
        allow_unavailable: bool,
    },
    /// Build a container image around a Linux single executable.
    Image {
        entry: Option<PathBuf>,
        #[arg(long, default_value = "tysel.toml")]
        manifest: PathBuf,
        /// Use an existing Linux executable instead of running `tysel build`.
        #[arg(long)]
        binary: Option<PathBuf>,
        #[arg(long)]
        stub: Option<PathBuf>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value = "dist/image")]
        output_dir: PathBuf,
        #[arg(long, default_value = "gcr.io/distroless/cc-debian13:nonroot")]
        base_image: String,
        /// Generate the Docker build context without invoking Docker.
        #[arg(long, alias = "no-build")]
        context_only: bool,
        /// Replace generated app and Dockerfile files in the output directory.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    if let Ok(capacity) = std::env::var("TYSEL_INTERNAL_TASK_MEMORY") {
        return match capacity
            .parse::<usize>()
            .context("parse TYSEL_INTERNAL_TASK_MEMORY")
            .and_then(tysel_testkit::task_backpressure_memory)
            .and_then(|report| serde_json::to_string(&report).context("encode task memory report"))
        {
            Ok(report) => {
                println!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    let cli = Cli::parse();
    let error_format = cli.error_format;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            match error_format {
                ErrorFormat::Human => eprintln!("error: {err:#}"),
                ErrorFormat::Json => eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": {
                            "code": "TYSEL_CLI_ERROR",
                            "message": format!("{err:#}"),
                        }
                    })
                ),
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
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
        Commands::Init { path } => init::run(&path),
        Commands::Test { paths, manifest, timeout_ms, json } => {
            test_runner::run(&manifest, &paths, timeout_ms, json)
        }
        Commands::Image {
            entry,
            manifest,
            binary,
            stub,
            tag,
            output_dir,
            base_image,
            context_only,
            force,
        } => image::run(image::Options {
            entry,
            manifest,
            binary,
            stub,
            tag,
            output_dir,
            base_image,
            context_only,
            force,
        }),
        Commands::Compat { manifest, json, strict, deny_unknown } => {
            compat::run(&manifest, json, strict, deny_unknown)
        }
        Commands::Bench { suite, format, evidence, source_commit, command, allow_unavailable } => {
            bench::run(bench::Options {
                suite,
                format,
                evidence,
                source_commit,
                command,
                allow_unavailable,
            })
        }
    }
}

fn inspect(path: &Path) -> Result<()> {
    let manifest =
        Manifest::from_path(path).with_context(|| format!("failed to read {}", path.display()))?;
    print!("{}", manifest.inspect_report());
    Ok(())
}
