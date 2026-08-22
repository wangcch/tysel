use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::{Value, json};
use tysel_testkit::{
    ARTIFACT_MB, BenchReport, BenchScale, COLD_START_MS, IDLE_MEMORY_MB, SuiteReport,
    benchmark_system, complete_benchmark_evidence, find_release_stub, find_release_worker,
    find_stub, gated_metric, run_durable, run_http, run_isolate, run_isolate_with_worker, run_task,
    suite_report, write_benchmark_evidence,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BenchSuite {
    Startup,
    Memory,
    Isolate,
    Task,
    Durable,
    Http,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BenchFormat {
    Human,
    Json,
}

pub struct Options {
    pub suite: BenchSuite,
    pub format: BenchFormat,
    pub evidence: Option<PathBuf>,
    pub source_commit: Option<String>,
    pub command: Option<String>,
    pub allow_unavailable: bool,
}

pub fn run(options: Options) -> Result<()> {
    validate_options(&options)?;
    let scale = BenchScale::from_env();
    let needs_baseline =
        matches!(options.suite, BenchSuite::Startup | BenchSuite::Memory | BenchSuite::All);
    let evidence_mode = options.evidence.is_some();
    let baseline = if needs_baseline {
        let stub = if evidence_mode { find_release_stub()? } else { find_stub()? };
        Some(tysel_testkit::measure(&stub).context("measure hello-service")?)
    } else {
        None
    };
    let release_worker = if evidence_mode { Some(find_release_worker()?) } else { None };
    let suites = run_suites(options.suite, baseline.as_ref(), scale, release_worker.as_deref())?;

    match options.format {
        BenchFormat::Human => print!("{}", format_human(&suites)),
        BenchFormat::Json => println!("{}", serde_json::to_string_pretty(&format_json(&suites))?),
    }

    if let Some(path) = &options.evidence {
        let baseline = baseline.as_ref().expect("all includes baseline measurement");
        if !suites_passed(&suites) {
            bail!("one or more benchmark release gates failed");
        }
        let source_commit = match options.source_commit {
            Some(value) => value,
            None => git_head().context("detect source commit for --evidence")?,
        };
        let command = options.command.unwrap_or_else(default_command);
        let evidence =
            complete_benchmark_evidence(baseline, suites.clone(), &source_commit, &command)?;
        write_benchmark_evidence(path, &evidence)?;
        eprintln!("Evidence             {}", path.display());
    }

    if !suites_passed(&suites) {
        bail!("one or more benchmark release gates failed");
    }
    Ok(())
}

fn validate_options(options: &Options) -> Result<()> {
    if options.evidence.is_none() && (options.source_commit.is_some() || options.command.is_some())
    {
        bail!("--source-commit and --command require --evidence");
    }
    if options.evidence.is_some() && options.suite != BenchSuite::All {
        bail!("benchmark evidence requires `tysel bench all`");
    }
    if options.allow_unavailable && options.suite != BenchSuite::All {
        bail!("--allow-unavailable is only valid with `tysel bench all`");
    }
    if options.allow_unavailable && options.evidence.is_some() {
        bail!("--allow-unavailable cannot be used with --evidence");
    }
    if options.evidence.is_some() && cfg!(debug_assertions) {
        bail!("benchmark evidence requires a release build (`cargo run --release ...`)");
    }
    if options.evidence.is_some()
        && matches!(std::env::var("TYSEL_BENCH_QUICK").as_deref(), Ok("1" | "true"))
    {
        bail!("benchmark evidence requires the full scale; unset TYSEL_BENCH_QUICK");
    }
    Ok(())
}

fn run_suites(
    requested: BenchSuite,
    baseline: Option<&BenchReport>,
    scale: BenchScale,
    release_worker: Option<&std::path::Path>,
) -> Result<Vec<SuiteReport>> {
    let mut suites = Vec::new();
    if matches!(requested, BenchSuite::Startup | BenchSuite::All) {
        suites.push(startup_report(baseline.expect("startup baseline")));
    }
    if matches!(requested, BenchSuite::Memory | BenchSuite::All) {
        let baseline = baseline.expect("memory baseline");
        suites.push(memory_report(baseline));
    }
    if requested == BenchSuite::All {
        let baseline = baseline.expect("artifact baseline");
        suites.push(artifact_report(baseline));
    }
    if matches!(requested, BenchSuite::Isolate | BenchSuite::All) {
        let report = match release_worker {
            Some(worker) => run_isolate_with_worker(scale, worker),
            None => run_isolate(scale),
        };
        suites.push(report.context("run isolate benchmark")?);
    }
    if matches!(requested, BenchSuite::Task | BenchSuite::All) {
        suites.push(run_task(scale).context("run task benchmark")?);
    }
    if matches!(requested, BenchSuite::Durable | BenchSuite::All) {
        suites.push(run_durable(scale).context("run durable benchmark")?);
    }
    if matches!(requested, BenchSuite::Http | BenchSuite::All) {
        suites.push(run_http(scale).context("run HTTP benchmark")?);
    }
    Ok(suites)
}

fn startup_report(report: &BenchReport) -> SuiteReport {
    suite_report(
        "startup",
        vec![gated_metric("cold_start_p50_ms", "ms", report.cold_start_ms.clone(), COLD_START_MS)],
    )
}

fn memory_report(report: &BenchReport) -> SuiteReport {
    let mut metric =
        gated_metric("idle_memory_mb", "MB", vec![report.idle_memory_mb()], IDLE_MEMORY_MB);
    metric.extra = Some(json!({ "kind": report.memory_kind }));
    if cfg!(target_os = "linux") && report.memory_kind != "pss" {
        metric.status = Some("fail".into());
        metric.passed = Some(false);
        metric.reason = Some("Linux release evidence requires PSS".into());
    }
    suite_report("memory", vec![metric])
}

fn artifact_report(report: &BenchReport) -> SuiteReport {
    let mut metric = gated_metric("artifact_mb", "MB", vec![report.artifact_mb()], ARTIFACT_MB);
    metric.extra = Some(json!({
        "bytes": report.artifact_bytes,
        "sha256": report.artifact_sha256,
    }));
    suite_report("binary-size", vec![metric])
}

fn suites_passed(suites: &[SuiteReport]) -> bool {
    suites.iter().flat_map(|suite| &suite.metrics).all(|metric| metric.passed != Some(false))
}

fn suite_status(suite: &SuiteReport) -> &'static str {
    if suite.metrics.iter().any(|metric| metric.passed == Some(false)) { "fail" } else { "pass" }
}

fn format_human(suites: &[SuiteReport]) -> String {
    let mut out = String::from(
        "suite             status    metric                              p50          p95          p99        gate\n",
    );
    for suite in suites {
        for (index, metric) in suite.metrics.iter().enumerate() {
            let suite_name = if index == 0 { suite.suite.as_str() } else { "" };
            let status = metric.status.as_deref().unwrap_or("observed");
            if status == "skipped" {
                out.push_str(&format!(
                    "{suite_name:<16}  {status:<8}  {:<34} {}\n",
                    metric.name,
                    metric.reason.as_deref().unwrap_or("not configured")
                ));
                continue;
            }
            let display = |value: Option<f64>| {
                value.map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".into())
            };
            let p50 = display(metric.p50);
            let p95 = display(metric.p95);
            let p99 = display(metric.p99);
            let gate = metric
                .limit
                .map(|limit| format!("≤ {limit:.2} {}", metric.unit))
                .unwrap_or_default();
            out.push_str(&format!(
                "{suite_name:<16}  {status:<8}  {:<34} {p50:>8} {:<4} {p95:>8} {:<4} {p99:>8} {:<4} {gate}\n",
                metric.name, metric.unit, metric.unit, metric.unit,
            ));
        }
    }
    out
}

fn format_json(suites: &[SuiteReport]) -> Value {
    json!({
        "schemaVersion": 2,
        "system": benchmark_system(),
        "suites": suites.iter().map(|suite| json!({
            "name": suite.suite,
            "status": suite_status(suite),
            "commit": suite.commit,
            "metrics": suite.metrics,
        })).collect::<Vec<_>>(),
    })
}

fn git_head() -> Result<String> {
    let root = tysel_testkit::workspace_root();
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(&root)
        .output()
        .context("git status")?;
    if !status.status.success() {
        bail!("git status failed; pass --source-commit");
    }
    if !status.stdout.is_empty() {
        bail!("workspace has uncommitted changes; pass --source-commit explicitly");
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .context("git rev-parse HEAD")?;
    if !output.status.success() {
        bail!("git rev-parse HEAD failed; pass --source-commit");
    }
    let commit = String::from_utf8(output.stdout)?.trim().to_owned();
    if commit.is_empty() {
        bail!("git HEAD is empty; pass --source-commit");
    }
    Ok(commit)
}

fn default_command() -> String {
    let mut args = std::env::args();
    let exe = args.next().map(PathBuf::from);
    let name = exe
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("tysel");
    let rest: Vec<String> = args.collect();
    if rest.is_empty() { format!("{name} bench all") } else { format!("{name} {}", rest.join(" ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(start_ms: f64, memory_kb: u64, artifact_bytes: u64) -> BenchReport {
        BenchReport {
            artifact_bytes,
            artifact_sha256: "ab".repeat(32),
            cold_start_ms: vec![start_ms; 11],
            idle_memory_kb: memory_kb,
            memory_kind: if cfg!(target_os = "linux") { "pss" } else { "rss" },
        }
    }

    #[test]
    fn baseline_reports_include_raw_samples_and_gates() {
        let report = sample_report(8.0, 10 * 1024, 1_000_000);
        let suites =
            vec![startup_report(&report), memory_report(&report), artifact_report(&report)];
        assert!(suites_passed(&suites));
        assert_eq!(format_json(&suites)["schemaVersion"], 2);
        assert_eq!(suites[0].metrics[0].samples.len(), 11);
        assert_eq!(suites[0].metrics[0].limit, Some(COLD_START_MS));
    }

    #[test]
    fn failed_gate_fails_suite_collection() {
        let report = sample_report(40.0, 10 * 1024, 1_000_000);
        let suites = vec![startup_report(&report)];
        assert!(!suites_passed(&suites));
        assert_eq!(suite_status(&suites[0]), "fail");
        assert!(format_human(&suites).contains("fail"));
    }

    #[test]
    fn observational_and_skipped_metrics_do_not_fake_failures() {
        let suite = suite_report(
            "task",
            vec![
                tysel_testkit::metric("enqueue_ms", "ms", vec![1.0]),
                tysel_testkit::skipped_metric("postgres_ms", "ms", "not configured"),
            ],
        );
        assert!(suites_passed(&[suite]));
    }
}
