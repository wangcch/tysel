use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::{Value, json};
use tysel_testkit::{
    ARTIFACT_MB, BenchReport, COLD_START_MS, IDLE_MEMORY_MB, benchmark_evidence, find_stub,
    gates_passed, write_benchmark_evidence,
};

const UNAVAILABLE_REASON: &str = "harness is not implemented yet";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BenchSuite {
    Startup,
    Memory,
    Isolate,
    Task,
    Durable,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SuiteStatus {
    Pass,
    Fail,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
struct SuiteResult {
    name: &'static str,
    status: SuiteStatus,
    reason: Option<&'static str>,
    measured: Option<f64>,
    unit: Option<&'static str>,
    limit: Option<f64>,
    samples: Option<Vec<f64>>,
    memory_kind: Option<&'static str>,
    bytes: Option<u64>,
    sha256: Option<String>,
}

pub fn run(options: Options) -> Result<()> {
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

    let names = requested_names(options.suite);
    if options.evidence.is_some() && names.iter().any(|name| !available_harness(name)) {
        bail!("benchmark evidence requires every requested suite to be available");
    }
    let needs_measure = names.iter().any(|name| available_harness(name));
    let report = if needs_measure {
        let stub = find_stub()?;
        Some(tysel_testkit::measure(&stub).context("measure hello-service")?)
    } else {
        None
    };

    let suites: Vec<SuiteResult> =
        names.iter().map(|name| suite_result(name, report.as_ref())).collect();
    match options.format {
        BenchFormat::Human => print!("{}", format_human(&suites)),
        BenchFormat::Json => println!("{}", serde_json::to_string_pretty(&format_json(&suites))?),
    }

    if let Some(path) = &options.evidence {
        let report = report.as_ref().expect("evidence requires a measurement");
        if !gates_passed(report) {
            bail!("one or more §30 gates failed");
        }
        let source_commit = match options.source_commit {
            Some(value) => value,
            None => git_head().context("detect source commit for --evidence")?,
        };
        let command = options.command.unwrap_or_else(default_command);
        let evidence = benchmark_evidence(report, &source_commit, &command)?;
        write_benchmark_evidence(path, &evidence)?;
        eprintln!("Evidence             {}", path.display());
    }

    if !command_succeeded(&suites, options.allow_unavailable) {
        if options.suite == BenchSuite::All {
            if suites.iter().any(|suite| suite.status == SuiteStatus::Unavailable) {
                bail!("one or more benchmark suites are unavailable");
            }
            bail!("one or more §30 gates failed");
        }
        if suites.iter().any(|suite| suite.status == SuiteStatus::Unavailable) {
            bail!("benchmark suite is unavailable");
        }
        bail!("benchmark suite failed");
    }
    Ok(())
}

fn requested_names(suite: BenchSuite) -> Vec<&'static str> {
    match suite {
        BenchSuite::Startup => vec!["startup"],
        BenchSuite::Memory => vec!["memory"],
        BenchSuite::Isolate => vec!["isolate"],
        BenchSuite::Task => vec!["task"],
        BenchSuite::Durable => vec!["durable"],
        BenchSuite::All => {
            vec!["startup", "memory", "binary-size", "isolate", "task", "durable"]
        }
    }
}

fn available_harness(name: &str) -> bool {
    matches!(name, "startup" | "memory" | "binary-size")
}

fn suite_result(name: &'static str, report: Option<&BenchReport>) -> SuiteResult {
    if !available_harness(name) {
        return SuiteResult {
            name,
            status: SuiteStatus::Unavailable,
            reason: Some(UNAVAILABLE_REASON),
            measured: None,
            unit: None,
            limit: None,
            samples: None,
            memory_kind: None,
            bytes: None,
            sha256: None,
        };
    }
    let report = report.expect("available suite requires a measurement");
    match name {
        "startup" => {
            let measured = report.cold_start_p50_ms();
            SuiteResult {
                name,
                status: if measured <= COLD_START_MS {
                    SuiteStatus::Pass
                } else {
                    SuiteStatus::Fail
                },
                reason: None,
                measured: Some(measured),
                unit: Some("ms"),
                limit: Some(COLD_START_MS),
                samples: Some(report.cold_start_ms.clone()),
                memory_kind: None,
                bytes: None,
                sha256: None,
            }
        }
        "memory" => {
            let measured = report.idle_memory_mb();
            let passed = measured <= IDLE_MEMORY_MB
                && (!cfg!(target_os = "linux") || report.memory_kind == "pss");
            SuiteResult {
                name,
                status: if passed { SuiteStatus::Pass } else { SuiteStatus::Fail },
                reason: None,
                measured: Some(measured),
                unit: Some("MB"),
                limit: Some(IDLE_MEMORY_MB),
                samples: None,
                memory_kind: Some(report.memory_kind),
                bytes: None,
                sha256: None,
            }
        }
        "binary-size" => {
            let measured = report.artifact_mb();
            SuiteResult {
                name,
                status: if measured <= ARTIFACT_MB { SuiteStatus::Pass } else { SuiteStatus::Fail },
                reason: None,
                measured: Some(measured),
                unit: Some("MB"),
                limit: Some(ARTIFACT_MB),
                samples: None,
                memory_kind: None,
                bytes: Some(report.artifact_bytes),
                sha256: Some(report.artifact_sha256.clone()),
            }
        }
        _ => unreachable!("available harness {name}"),
    }
}

fn command_succeeded(results: &[SuiteResult], allow_unavailable: bool) -> bool {
    if results.iter().any(|result| result.status == SuiteStatus::Fail) {
        return false;
    }
    if !allow_unavailable && results.iter().any(|result| result.status == SuiteStatus::Unavailable)
    {
        return false;
    }
    true
}

fn format_human(results: &[SuiteResult]) -> String {
    let mut out = String::from("suite             status        measured           limit\n");
    for result in results {
        match result.status {
            SuiteStatus::Unavailable => {
                out.push_str(&format!(
                    "{:<16}  unavailable   {}\n",
                    result.name,
                    result.reason.unwrap_or(UNAVAILABLE_REASON)
                ));
            }
            SuiteStatus::Pass | SuiteStatus::Fail => {
                let status = if result.status == SuiteStatus::Pass { "pass" } else { "fail" };
                let measured = result.measured.unwrap_or(0.0);
                let unit = result.unit.unwrap_or("");
                let limit = result.limit.unwrap_or(0.0);
                let kind = result.memory_kind.map(|kind| format!(" ({kind})")).unwrap_or_default();
                out.push_str(&format!(
                    "{:<16}  {status:<12}  {measured:>8.2} {unit:<4}  {limit:>6} {unit}{kind}\n",
                    result.name
                ));
            }
        }
    }
    out
}

fn format_json(results: &[SuiteResult]) -> Value {
    json!({
        "schemaVersion": 1,
        "suites": results.iter().map(suite_json).collect::<Vec<_>>(),
    })
}

fn suite_json(result: &SuiteResult) -> Value {
    match result.status {
        SuiteStatus::Unavailable => json!({
            "name": result.name,
            "status": "unavailable",
            "reason": result.reason.unwrap_or(UNAVAILABLE_REASON),
        }),
        SuiteStatus::Pass | SuiteStatus::Fail => {
            let mut body = json!({
                "name": result.name,
                "status": if result.status == SuiteStatus::Pass { "pass" } else { "fail" },
                "measured": result.measured,
                "unit": result.unit,
                "limit": result.limit,
            });
            let object = body.as_object_mut().expect("suite object");
            if let Some(samples) = &result.samples {
                object.insert("samples".into(), json!(samples));
            }
            if let Some(kind) = result.memory_kind {
                object.insert("kind".into(), json!(kind));
            }
            if let Some(bytes) = result.bytes {
                object.insert("bytes".into(), json!(bytes));
            }
            if let Some(digest) = &result.sha256 {
                object.insert("sha256".into(), json!(digest));
            }
            body
        }
    }
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
    fn all_includes_unavailable_suites_without_numbers() {
        let report = sample_report(8.0, 10 * 1024, 1_000_000);
        let suites: Vec<_> = requested_names(BenchSuite::All)
            .into_iter()
            .map(|name| suite_result(name, Some(&report)))
            .collect();
        assert_eq!(suites.len(), 6);
        assert_eq!(suites[0].name, "startup");
        assert_eq!(suites[0].status, SuiteStatus::Pass);
        assert_eq!(suites[3].name, "isolate");
        assert_eq!(suites[3].status, SuiteStatus::Unavailable);
        assert!(suites[3].measured.is_none());
        let json = format_json(&suites);
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["suites"][3]["status"], "unavailable");
        assert!(json["suites"][3].get("measured").is_none());
        assert!(json["suites"][3].get("samples").is_none());
        assert!(!command_succeeded(&suites, false));
        assert!(command_succeeded(&suites, true));
    }

    #[test]
    fn explicit_unavailable_suite_fails() {
        let suites = vec![suite_result("isolate", None)];
        assert!(!command_succeeded(&suites, false));
        let json = format_json(&suites);
        assert_eq!(json["suites"][0]["reason"], UNAVAILABLE_REASON);
    }

    #[test]
    fn failed_gate_fails_all() {
        let report = sample_report(40.0, 10 * 1024, 1_000_000);
        let suites: Vec<_> = requested_names(BenchSuite::All)
            .into_iter()
            .map(|name| suite_result(name, Some(&report)))
            .collect();
        assert_eq!(suites[0].status, SuiteStatus::Fail);
        assert!(!command_succeeded(&suites, false));
        assert!(!command_succeeded(&suites, true));
        let human = format_human(&suites);
        assert!(human.contains("fail"));
        assert!(human.contains("unavailable"));
    }

    #[test]
    fn json_omits_fake_fields_for_unavailable_suites() {
        let text = serde_json::to_string(&format_json(&[suite_result("durable", None)])).unwrap();
        assert!(!text.contains("measured"));
        assert!(!text.contains("samples"));
        assert!(!text.contains("limit"));
    }
}
