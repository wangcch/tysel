use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::Serialize;
use tysel_bench_compare::{COMPARISON_SUMMARY_SCHEMA_VERSION, ComparisonSummary, workspace_root};

#[derive(Debug, Parser)]
#[command(about = "Check three four-seed record cycles for publication stability")]
struct Cli {
    #[arg(long, required = true, num_args = 3)]
    input: Vec<PathBuf>,
    #[arg(long, default_value = "target/benchmark-comparison/stability-v1.json")]
    output: PathBuf,
    #[arg(long, default_value_t = 10.0)]
    primary_threshold_pct: f64,
    #[arg(long, default_value_t = 15.0)]
    guardrail_threshold_pct: f64,
    #[arg(long)]
    fail_on_unstable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StabilityEvidence {
    schema_version: u32,
    source_commit: String,
    architecture: String,
    stable: bool,
    primary_metric: String,
    inputs: Vec<String>,
    checks: Vec<StabilityCheck>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StabilityCheck {
    runtime: String,
    metric: String,
    workload: String,
    concurrency: usize,
    values: Vec<f64>,
    relative_spread_pct: f64,
    threshold_pct: f64,
    stable: bool,
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
    ensure!(cli.primary_threshold_pct >= 0.0, "primary threshold must be non-negative");
    ensure!(cli.guardrail_threshold_pct >= 0.0, "guardrail threshold must be non-negative");
    let root = workspace_root();
    let paths = cli.input.iter().map(|path| resolve_path(&root, path)).collect::<Vec<_>>();
    let summaries = paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_slice::<ComparisonSummary>(&bytes)
                .with_context(|| format!("parse {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    validate_cycles(&summaries)?;
    let checks =
        stability_checks(&summaries, cli.primary_threshold_pct, cli.guardrail_threshold_pct)?;
    let stable = checks.iter().all(|check| check.stable);
    let evidence = StabilityEvidence {
        schema_version: 1,
        source_commit: summaries[0].source_commit.clone(),
        architecture: summaries[0].system.arch.clone(),
        stable,
        primary_metric: "requests-per-server-cpu-second-p50".into(),
        inputs: paths.iter().map(|path| path.display().to_string()).collect(),
        checks,
    };
    let output = resolve_path(&root, &cli.output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut json = serde_json::to_vec_pretty(&evidence)?;
    json.push(b'\n');
    fs::write(&output, json).with_context(|| format!("write {}", output.display()))?;
    println!("Stability {}", output.display());
    for check in evidence.checks.iter().filter(|check| !check.stable) {
        eprintln!(
            "unstable: runtime={} metric={} workload={} concurrency={} values={:?} spread={:.2}% threshold={:.2}%",
            check.runtime,
            check.metric,
            check.workload,
            check.concurrency,
            check.values,
            check.relative_spread_pct,
            check.threshold_pct
        );
    }
    ensure!(!cli.fail_on_unstable || stable, "record cycles are unstable");
    Ok(())
}

fn validate_cycles(summaries: &[ComparisonSummary]) -> Result<()> {
    ensure!(summaries.len() == 3, "exactly three record cycles are required");
    let first = &summaries[0];
    ensure!(first.schema_version == COMPARISON_SUMMARY_SCHEMA_VERSION, "unsupported schema");
    ensure!(!first.quick, "quick summaries are not publication evidence");
    ensure!(first.inputs.len() == 4, "each record cycle must aggregate four seeds");
    ensure!(first.order_seeds == [1, 2, 3, 4], "each cycle must contain seeds 1,2,3,4");
    for summary in &summaries[1..] {
        ensure!(summary.schema_version == first.schema_version, "summary schema differs");
        ensure!(!summary.quick, "quick summaries are not publication evidence");
        ensure!(summary.inputs.len() == 4, "each record cycle must aggregate four seeds");
        ensure!(summary.order_seeds == [1, 2, 3, 4], "each cycle must contain seeds 1,2,3,4");
        ensure!(
            summary.source_commit == first.source_commit,
            "source commit differs across cycles"
        );
        ensure!(summary.matrix == first.matrix, "benchmark matrix differs across cycles");
        ensure!(summary.runtime_lock == first.runtime_lock, "runtime lock differs across cycles");
        ensure!(
            serde_json::to_value(&summary.system)? == serde_json::to_value(&first.system)?,
            "system fingerprint differs across cycles"
        );
        ensure!(
            serde_json::to_value(&summary.toolchains)? == serde_json::to_value(&first.toolchains)?,
            "source toolchain differs across cycles"
        );
        let identities = |value: &ComparisonSummary| {
            value
                .runtimes
                .iter()
                .map(|runtime| {
                    (
                        runtime.id.clone(),
                        runtime.actual_version.clone(),
                        runtime.build_mode.clone(),
                        runtime.executable_sha256.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        ensure!(
            identities(summary) == identities(first),
            "runtime identities differ across cycles"
        );
    }
    Ok(())
}

fn stability_checks(
    summaries: &[ComparisonSummary],
    primary_threshold_pct: f64,
    guardrail_threshold_pct: f64,
) -> Result<Vec<StabilityCheck>> {
    let mut checks = Vec::new();
    for runtime in &summaries[0].runtimes {
        for workload in &runtime.workloads {
            push_check(
                &mut checks,
                summaries,
                &runtime.id,
                workload,
                "requests-per-server-cpu-second-p50",
                primary_threshold_pct,
                |candidate| {
                    candidate.requests_per_server_cpu_second.as_ref().map(|value| value.p50)
                },
            )?;
            push_check(
                &mut checks,
                summaries,
                &runtime.id,
                workload,
                "requests-per-second-p50",
                guardrail_threshold_pct,
                |candidate| Some(candidate.requests_per_second.p50),
            )?;
            push_check(
                &mut checks,
                summaries,
                &runtime.id,
                workload,
                "latency-p50-ms",
                guardrail_threshold_pct,
                |candidate| Some(candidate.latency_ms.p50),
            )?;
        }
    }
    Ok(checks)
}

fn push_check<F>(
    checks: &mut Vec<StabilityCheck>,
    summaries: &[ComparisonSummary],
    runtime_id: &str,
    workload: &tysel_bench_compare::HttpWorkloadSummary,
    metric: &str,
    threshold_pct: f64,
    value: F,
) -> Result<()>
where
    F: Fn(&tysel_bench_compare::HttpWorkloadSummary) -> Option<f64>,
{
    let values = summaries
        .iter()
        .map(|summary| {
            let runtime = summary.runtimes.iter().find(|runtime| runtime.id == runtime_id)?;
            let candidate = runtime.workloads.iter().find(|candidate| {
                candidate.id == workload.id && candidate.concurrency == workload.concurrency
            })?;
            value(candidate)
        })
        .collect::<Option<Vec<_>>>()
        .with_context(|| {
            format!(
                "missing {metric} for {runtime_id} {}/{} in one or more cycles",
                workload.id, workload.concurrency,
            )
        })?;
    let spread = relative_spread_pct(&values);
    checks.push(StabilityCheck {
        runtime: runtime_id.into(),
        metric: metric.into(),
        workload: workload.id.clone(),
        concurrency: workload.concurrency,
        values,
        relative_spread_pct: spread,
        threshold_pct,
        stable: spread <= threshold_pct,
    });
    Ok(())
}

fn relative_spread_pct(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    if median == 0.0 {
        return if sorted.first() == sorted.last() { 0.0 } else { f64::INFINITY };
    }
    (sorted.last().unwrap() - sorted.first().unwrap()) / median.abs() * 100.0
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_owned() } else { root.join(path) }
}

#[cfg(test)]
mod tests {
    use super::relative_spread_pct;

    #[test]
    fn spread_is_range_over_median() {
        assert!((relative_spread_pct(&[95.0, 100.0, 105.0]) - 10.0).abs() < f64::EPSILON);
    }
}
