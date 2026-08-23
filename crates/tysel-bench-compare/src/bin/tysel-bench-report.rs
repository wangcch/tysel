use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use tysel_bench_compare::{
    COMPARISON_SUMMARY_SCHEMA_VERSION, ComparisonEvidence, ComparisonSummary, SummaryInput,
    aggregate_comparisons, compare_to_baseline, now_unix_ms, render_summary_markdown, sha256_file,
    workspace_root,
};

#[derive(Debug, Parser)]
#[command(about = "Aggregate rotated Tysel/Node/Bun/Deno evidence and evaluate regressions")]
struct Cli {
    #[arg(long, required = true, num_args = 1..)]
    input: Vec<PathBuf>,
    #[arg(long, default_value = "target/benchmark-comparison/summary-v1.json")]
    output: PathBuf,
    #[arg(long, default_value_t = 4)]
    required_runs: usize,
    #[arg(long)]
    allow_quick: bool,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long, default_value_t = 5.0)]
    regression_threshold_pct: f64,
    #[arg(long)]
    fail_on_regression: bool,
    #[arg(long, value_delimiter = ',', default_value = "tysel")]
    gate_runtime: Vec<String>,
    #[arg(long, value_delimiter = ',', default_value = "requests-per-server-cpu-second-p50")]
    gate_metric: Vec<String>,
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let root = workspace_root();
    let output = resolve_path(&root, &cli.output);
    ensure!(!cli.gate_runtime.is_empty(), "gate_runtime must not be empty");
    ensure!(!cli.gate_metric.is_empty(), "gate_metric must not be empty");
    let mut inputs = Vec::with_capacity(cli.input.len());
    for path in &cli.input {
        let path = resolve_path(&root, path);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let evidence: ComparisonEvidence =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        inputs.push((
            SummaryInput {
                path: relative_label(&root, &path),
                run_id: evidence.run_id.clone(),
                sha256: sha256_file(&path)?,
            },
            evidence,
        ));
    }
    let mut summary = aggregate_comparisons(
        inputs,
        cli.required_runs,
        cli.allow_quick,
        cli.allow_dirty,
        now_unix_ms()?,
    )?;
    if let Some(path) = &cli.baseline {
        let path = resolve_path(&root, path);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let baseline: ComparisonSummary =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        ensure!(
            baseline.schema_version == COMPARISON_SUMMARY_SCHEMA_VERSION,
            "unsupported baseline summary schemaVersion {}",
            baseline.schema_version
        );
        summary.baseline =
            Some(compare_to_baseline(&summary, &baseline, cli.regression_threshold_pct)?);
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut json = serde_json::to_vec_pretty(&summary)?;
    json.push(b'\n');
    fs::write(&output, json).with_context(|| format!("write {}", output.display()))?;
    let report = output.with_extension("md");
    fs::write(&report, render_summary_markdown(&summary))
        .with_context(|| format!("write {}", report.display()))?;
    println!("Summary   {}", output.display());
    println!("Report    {}", report.display());

    if cli.fail_on_regression {
        let baseline =
            summary.baseline.as_ref().context("--fail-on-regression requires --baseline")?;
        let regressions: Vec<_> = baseline
            .changes
            .iter()
            .filter(|change| {
                change.classification == "regression"
                    && cli.gate_runtime.contains(&change.runtime)
                    && (cli.gate_metric.iter().any(|metric| metric == "all")
                        || cli.gate_metric.contains(&change.metric))
            })
            .collect();
        ensure!(
            regressions.is_empty(),
            "{} gated regression(s) exceeded ±{:.1}% for metric filter {:?}",
            regressions.len(),
            cli.regression_threshold_pct,
            cli.gate_metric
        );
    }
    Ok(())
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_owned() } else { root.join(path) }
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}
