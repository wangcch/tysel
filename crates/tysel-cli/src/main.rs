//! Tysel CLI.
//!
//! `inspect` and `build` ship a TAP trailer. `check` validates a project.
//! `dev` watches sources and serves with process-level reload.
//! `run` serves the same way without watching files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{fs, io::Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use tysel_manifest::ManifestFormat;

mod bench;
mod build;
mod check;
mod compat;
mod cross_target;
mod dev;
mod doctor;
mod image;
mod init;
mod integrity;
mod node_scan;
mod platform;
mod project;
mod release;
mod task;
mod test_runner;
mod typegen;
mod upgrade;

#[derive(Parser)]
#[command(
    name = "tysel",
    version,
    about = "A lightweight native TypeScript runtime for services and AI agents.",
    arg_required_else_help = true
)]
struct Cli {
    /// Format stderr errors and development diagnostics for humans or automation.
    #[arg(long, global = true, value_enum, default_value_t = ErrorFormat::Human)]
    error_format: ErrorFormat,
    /// Discover a Tysel project as if invoked from this directory; doctor also accepts a manifest.
    #[arg(short = 'C', long = "project", visible_alias = "project-dir", global = true)]
    project_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ErrorFormat {
    Human,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConfigFormatArg {
    Toml,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum PackageJsonArg {
    Auto,
    Create,
    Reuse,
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum PackageManagerArg {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

#[derive(Clone, Copy, ValueEnum)]
enum InitTemplateArg {
    Http,
    Worker,
    Mcp,
    Minimal,
}

impl From<ConfigFormatArg> for ManifestFormat {
    fn from(value: ConfigFormatArg) -> Self {
        match value {
            ConfigFormatArg::Toml => Self::Toml,
            ConfigFormatArg::Json => Self::Json,
        }
    }
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the discovered manifest path.
    Path,
    /// Validate the manifest and report the resolved project context.
    Validate,
    /// Print the effective manifest with defaults expanded.
    Show {
        /// Select the output serialization; defaults to the source format.
        #[arg(long, value_enum)]
        format: Option<ConfigFormatArg>,
    },
    /// Serialize the effective manifest in another supported format.
    Convert {
        #[arg(long, value_enum)]
        to: ConfigFormatArg,
        /// Create this file instead of writing to stdout; existing files are preserved.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print the bundled JSON Schema for Tysel manifests.
    Schema,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Tysel application.
    Init {
        /// Project directory; omit it in a terminal to choose interactively.
        path: Option<PathBuf>,
        /// Select the generated application template.
        #[arg(long, value_enum)]
        template: Option<InitTemplateArg>,
        /// Select the generated Tysel manifest format.
        #[arg(long, value_enum)]
        manifest_format: Option<ConfigFormatArg>,
        /// Use an existing entry or choose the path for a generated entry.
        #[arg(long)]
        entry: Option<PathBuf>,
        /// Control package.json creation or reuse.
        #[arg(long, value_enum)]
        package_json: Option<PackageJsonArg>,
        /// Add namespaced Tysel scripts to a reused package.json.
        #[arg(long)]
        add_scripts: bool,
        /// Select the package manager used for dependency installation instructions.
        #[arg(long, value_enum)]
        package_manager: Option<PackageManagerArg>,
        /// Install generated package dependencies after creating the project.
        #[arg(long)]
        install: bool,
        /// Run `tysel check` after creation and optional dependency installation.
        #[arg(long)]
        verify: bool,
        /// Do not generate an application test.
        #[arg(long)]
        no_tests: bool,
        /// Print the planned file changes without writing them.
        #[arg(long)]
        dry_run: bool,
        /// Serialize a dry-run plan as JSON, including before/after file contents.
        #[arg(long, requires = "dry_run")]
        json: bool,
        /// Include full unified file diffs in a human-readable dry run.
        #[arg(long, requires = "dry_run", conflicts_with = "json")]
        diff: bool,
        /// Accept recommended defaults and never prompt.
        #[arg(short = 'y', long)]
        yes: bool,
        /// Disable prompts; omitted choices use documented defaults.
        #[arg(long, conflicts_with = "yes")]
        no_interactive: bool,
    },
    /// Discover, validate, and inspect project configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
        #[arg(long, global = true)]
        manifest: Option<PathBuf>,
    },
    /// List or run reproducible tasks declared in the Tysel manifest.
    Task {
        /// Task name; omit it to list available tasks.
        name: Option<String>,
        /// List tasks even when a name would otherwise be expected.
        #[arg(long, conflicts_with = "name")]
        list: bool,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Watch, bundle, and run a service with reload.
    Dev {
        entry: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Type-check, capability-scan, and validate the manifest.
    Check {
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Generate a manifest-scoped TypeScript capability environment.
    Types {
        /// Create this declaration file relative to the project root.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Fail when the generated declaration is missing or out of date.
        #[arg(long)]
        check: bool,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Run an application without watching files for reload.
    Run {
        entry: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Serve registered MCP tools over newline-delimited stdio.
    Mcp {
        entry: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Submit one message to a registered Queue handler and print its result.
    Queue {
        name: String,
        #[arg(long, default_value = "null")]
        input: String,
        #[arg(long)]
        message_id: Option<String>,
        entry: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Run application tests in isolated QuickJS instances.
    Test {
        /// Test files or directories (defaults to tests/).
        paths: Vec<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Timeout for each test.
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        /// Emit a stable machine-readable report.
        #[arg(long)]
        json: bool,
        /// List discovered tests without running their bodies.
        #[arg(long)]
        list: bool,
        /// Run tests whose name contains this text, or the test with this discovery ID.
        #[arg(short = 't', long)]
        filter: Option<String>,
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
        /// Require a previously verified target runtime; never access the network.
        #[arg(long)]
        offline: bool,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Sign or verify release evidence and inspect release keys.
    Release {
        #[command(subcommand)]
        command: release::ReleaseCommand,
    },
    /// Diagnose the installation, platform, and project without modifying them.
    Doctor {
        /// Run only checks suitable for installer preflight.
        #[arg(long)]
        install: bool,
        /// Explicitly enable release-network diagnostics.
        #[arg(long)]
        network: bool,
        /// Emit a stable machine-readable report.
        #[arg(long)]
        json: bool,
    },
    /// Safely check, upgrade, or roll back a managed Tysel installation.
    Upgrade {
        /// Check for a newer release without modifying the installation.
        #[arg(long)]
        check: bool,
        /// Select an immutable release version.
        #[arg(long, conflicts_with = "channel")]
        version: Option<String>,
        /// Select and persist the stable or canary release channel.
        #[arg(long)]
        channel: Option<String>,
        /// Confirm mutation without an interactive prompt.
        #[arg(long)]
        yes: bool,
        /// Permit a downgrade or reinstall of the active version.
        #[arg(long)]
        force: bool,
        /// Activate the retained previous release.
        #[arg(long)]
        rollback: bool,
        /// Emit a stable machine-readable report.
        #[arg(long)]
        json: bool,
    },
    /// Print the effective capability and permission report.
    Inspect {
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Report npm/Web API compatibility for the current project.
    Compat {
        #[arg(long)]
        manifest: Option<PathBuf>,
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
    /// Run the benchmark and release-gate harness.
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
        #[arg(long)]
        manifest: Option<PathBuf>,
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
        /// Container builder executable. Defaults to $DOCKER, then docker.
        #[arg(long)]
        builder: Option<PathBuf>,
        /// Verify and copy the existing binary's release sidecars into the context.
        #[arg(long, requires = "binary")]
        copy_sidecars: bool,
        /// Application version written to org.opencontainers.image.version.
        #[arg(long)]
        image_version: Option<String>,
        /// Additional OCI image label in KEY=VALUE form.
        #[arg(long, value_name = "KEY=VALUE")]
        label: Vec<String>,
        /// Generate the Docker build context without invoking Docker.
        #[arg(long, alias = "no-build")]
        context_only: bool,
        /// Replace generated app and Dockerfile files in the output directory.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    if let Some(output) = tysel_distribution::metadata_output("tysel", env!("CARGO_PKG_VERSION")) {
        println!("{output}");
        return ExitCode::SUCCESS;
    }
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
        Ok(code) => code,
        Err(err) => {
            match error_format {
                ErrorFormat::Human => eprintln!("error: {err:#}"),
                ErrorFormat::Json => {
                    let mut output = serde_json::json!({
                        "error": {
                            "code": "TYSEL_CLI_ERROR",
                            "message": format!("{err:#}"),
                        }
                    });
                    if let Some(diagnostics) = structured_diagnostics(&err) {
                        output["diagnostics"] = serde_json::json!(diagnostics.diagnostics());
                    }
                    eprintln!("{output}");
                }
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let error_format = cli.error_format;
    let invocation_dir = std::env::current_dir().context("resolve current directory")?;
    let project_dir = cli
        .project_dir
        .map(|path| if path.is_absolute() { path } else { invocation_dir.join(path) });
    let context = |manifest: Option<&Path>| -> Result<project::ProjectContext> {
        let project = project::ProjectContext::discover(project_dir.as_deref(), manifest)?;
        std::env::set_current_dir(&project.root)
            .with_context(|| format!("switch to project directory {}", project.root.display()))?;
        Ok(project)
    };
    let result = match cli.command {
        Commands::Inspect { manifest } => inspect(&context(manifest.as_deref())?),
        Commands::Check { manifest } => {
            let project = context(manifest.as_deref())?;
            check::run(&project.manifest_path)
        }
        Commands::Types { output, check, manifest } => {
            let project = context(manifest.as_deref())?;
            typegen::run(&project, output.as_deref(), check)
        }
        Commands::Dev { entry, manifest } => {
            let project = context(manifest.as_deref())?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run(project.manifest_path, entry, error_format))
        }
        Commands::Run { entry, manifest } => {
            let project = context(manifest.as_deref())?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_once(project.manifest_path, entry))
        }
        Commands::Mcp { entry, manifest } => {
            let project = context(manifest.as_deref())?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_mcp(project.manifest_path, entry))
        }
        Commands::Queue { name, input, message_id, entry, manifest } => {
            let project = context(manifest.as_deref())?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            runtime.block_on(dev::run_queue(project.manifest_path, entry, name, message_id, input))
        }
        Commands::Build { entry, stub, offline, output, manifest, target, profile, release } => {
            let project = context(manifest.as_deref())?;
            build::run(
                project.manifest_path,
                build::Options { entry, stub, offline, output, target, profile, release },
            )
        }
        Commands::Release { command } => {
            switch_to_selected_dir(project_dir.as_deref())?;
            release::run(command)
        }
        Commands::Doctor { install, network, json } => {
            if project_dir.as_deref().is_some_and(Path::is_dir) {
                switch_to_selected_dir(project_dir.as_deref())?;
            }
            let healthy = doctor::run(doctor::Options {
                project: project_dir.clone(),
                install_only: install,
                network,
                json,
            })?;
            return Ok(if healthy { ExitCode::SUCCESS } else { ExitCode::from(1) });
        }
        Commands::Upgrade { check, version, channel, yes, force, rollback, json } => {
            switch_to_selected_dir(project_dir.as_deref())?;
            upgrade::run(upgrade::Options { check, version, channel, yes, force, rollback, json })
        }
        Commands::Init {
            path,
            template,
            manifest_format,
            entry,
            package_json,
            add_scripts,
            package_manager,
            install,
            verify,
            no_tests,
            dry_run,
            json,
            diff,
            yes,
            no_interactive,
        } => {
            let path = match (path, project_dir.as_deref()) {
                (Some(path), Some(base)) if path == Path::new(".") => Some(base.to_path_buf()),
                (Some(path), Some(base)) if path.is_relative() => Some(base.join(path)),
                (Some(path), _) => Some(path),
                (None, Some(base)) => Some(base.to_path_buf()),
                (None, None) => None,
            };
            init::run(init::Request {
                root: path,
                template: template.map(|value| match value {
                    InitTemplateArg::Http => init::Template::Http,
                    InitTemplateArg::Worker => init::Template::Worker,
                    InitTemplateArg::Mcp => init::Template::Mcp,
                    InitTemplateArg::Minimal => init::Template::Minimal,
                }),
                manifest_format: manifest_format.map(ManifestFormat::from),
                entry,
                package_json: package_json.map(|value| match value {
                    PackageJsonArg::Auto => init::PackageJsonMode::Auto,
                    PackageJsonArg::Create => init::PackageJsonMode::Create,
                    PackageJsonArg::Reuse => init::PackageJsonMode::Reuse,
                    PackageJsonArg::None => init::PackageJsonMode::None,
                }),
                add_scripts,
                package_manager: package_manager.map(|value| match value {
                    PackageManagerArg::Npm => init::PackageManager::Npm,
                    PackageManagerArg::Pnpm => init::PackageManager::Pnpm,
                    PackageManagerArg::Yarn => init::PackageManager::Yarn,
                    PackageManagerArg::Bun => init::PackageManager::Bun,
                }),
                install: install.then_some(true),
                verify: verify.then_some(true),
                include_tests: no_tests.then_some(false),
                dry_run,
                json,
                diff,
                yes,
                no_interactive,
            })
        }
        Commands::Config { command: ConfigCommand::Schema, manifest } => {
            if manifest.is_some() {
                return Err(anyhow::anyhow!("config schema does not accept --manifest"));
            }
            switch_to_selected_dir(project_dir.as_deref())?;
            print!("{}", tysel_manifest::JSON_SCHEMA);
            Ok(())
        }
        Commands::Config { command: ConfigCommand::Path, manifest } => {
            let location =
                project::ProjectLocation::discover(project_dir.as_deref(), manifest.as_deref())?;
            println!("{}", location.manifest_path.display());
            Ok(())
        }
        Commands::Config { command, manifest } => {
            let project = context(manifest.as_deref())?;
            run_config(command, &project)
        }
        Commands::Task { name, list, manifest } => {
            let project = context(manifest.as_deref())?;
            task::run(&project, name.as_deref(), list)
        }
        Commands::Test { paths, manifest, timeout_ms, json, list, filter } => {
            let project = context(manifest.as_deref())?;
            test_runner::run(
                &project.manifest_path,
                &paths,
                timeout_ms,
                json,
                list,
                filter.as_deref(),
            )
        }
        Commands::Image {
            entry,
            manifest,
            binary,
            stub,
            tag,
            output_dir,
            base_image,
            builder,
            copy_sidecars,
            image_version,
            label,
            context_only,
            force,
        } => image::run(image::Options {
            entry,
            manifest: context(manifest.as_deref())?.manifest_path,
            binary,
            stub,
            tag,
            output_dir,
            base_image,
            builder,
            copy_sidecars,
            image_version,
            labels: label,
            context_only,
            force,
        }),
        Commands::Compat { manifest, json, strict, deny_unknown } => {
            let project = context(manifest.as_deref())?;
            compat::run(&project.manifest_path, json, strict, deny_unknown)
        }
        Commands::Bench { suite, format, evidence, source_commit, command, allow_unavailable } => {
            switch_to_selected_dir(project_dir.as_deref())?;
            bench::run(bench::Options {
                suite,
                format,
                evidence,
                source_commit,
                command,
                allow_unavailable,
            })
        }
    };
    result?;
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn structured_diagnostics(
    error: &anyhow::Error,
) -> Option<&tysel_build::BuildDiagnostics> {
    error.chain().find_map(|cause| cause.downcast_ref())
}

fn switch_to_selected_dir(path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        std::env::set_current_dir(path)
            .with_context(|| format!("switch to selected directory {}", path.display()))?;
    }
    Ok(())
}

fn inspect(project: &project::ProjectContext) -> Result<()> {
    print!("{}", project.manifest.inspect_report());
    Ok(())
}

fn run_config(command: ConfigCommand, project: &project::ProjectContext) -> Result<()> {
    match command {
        ConfigCommand::Path => unreachable!("path does not require a loaded project context"),
        ConfigCommand::Validate => {
            println!("Manifest     {}", project.manifest_path.display());
            println!("Format       {}", project.manifest_format.label());
            println!("Project root {}", project.root.display());
            println!(
                "Package      {}",
                project
                    .package_json
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".into())
            );
            println!("Status       valid");
        }
        ConfigCommand::Show { format } => {
            let format = format.map(ManifestFormat::from).unwrap_or(project.manifest_format);
            println!("{}", project.manifest.to_string_pretty(format)?);
        }
        ConfigCommand::Convert { to, output } => {
            let rendered = project.manifest.to_string_pretty(to.into())?;
            if let Some(path) = output {
                let requested = if path.is_absolute() { path } else { project.root.join(path) };
                let path = resolve_create_target(&requested)?;
                let output_format = ManifestFormat::from_path(&path)?;
                if output_format != ManifestFormat::from(to) {
                    return Err(anyhow::anyhow!(
                        "output extension does not match --to {}: {}",
                        ManifestFormat::from(to).label(),
                        path.display()
                    ));
                }
                if path.parent() == Some(project.root.as_path())
                    && project::MANIFEST_NAMES
                        .iter()
                        .any(|name| path.file_name() == Some(std::ffi::OsStr::new(name)))
                    && path != project.manifest_path
                {
                    return Err(anyhow::anyhow!(
                        "refusing to create {} beside {}; that would make project discovery ambiguous",
                        path.display(),
                        project.manifest_path.display()
                    ));
                }
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .with_context(|| format!("create {}", path.display()))?;
                let write_result =
                    file.write_all(rendered.as_bytes()).and_then(|()| file.write_all(b"\n"));
                if let Err(error) = write_result {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error).with_context(|| format!("write {}", path.display()));
                }
                println!("created {}", path.display());
            } else {
                println!("{rendered}");
            }
        }
        ConfigCommand::Schema => unreachable!("schema does not require a project context"),
    }
    Ok(())
}

fn resolve_create_target(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("output must name a file: {}", path.display()))?;
    let parent = path.parent().unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("resolve output directory {}", parent.display()))?;
    Ok(parent.join(file_name))
}
