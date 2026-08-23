use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const COMPARISON_SCHEMA_VERSION: u32 = 1;
pub const COMPARISON_SUMMARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSystem {
    pub os: String,
    pub arch: String,
    pub os_version: String,
    pub cpu_model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub schema_version: u32,
    pub measurement: MeasurementConfig,
    pub workloads: Vec<WorkloadConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementConfig {
    pub startup_warmups: usize,
    pub startup_samples: usize,
    pub idle_settle_ms: u64,
    pub http_warmup_requests: usize,
    pub http_rounds: usize,
    pub http_round_duration_ms: u64,
    pub concurrency: Vec<usize>,
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadConfig {
    pub id: String,
    pub path: String,
    pub response: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeLock {
    pub schema_version: u32,
    #[serde(default)]
    pub prepare: Vec<CommandSpec>,
    #[serde(default)]
    pub toolchains: Vec<ToolchainSpec>,
    pub runtimes: Vec<RuntimeSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolchainSpec {
    pub id: String,
    pub expected_version: String,
    pub version_command: CommandSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSpec {
    pub id: String,
    pub expected_version: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub readiness_prefix: String,
    pub build_mode: String,
    #[serde(default)]
    pub prepare: Vec<CommandSpec>,
    pub version_command: Option<CommandSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandSpec {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComparisonEvidence {
    pub schema_version: u32,
    pub run_id: String,
    pub generated_at_unix_ms: u128,
    pub source_commit: String,
    pub workspace_dirty: bool,
    pub command: String,
    pub matrix: String,
    pub runtime_lock: String,
    pub quick: bool,
    pub order_seed: u64,
    pub system: BenchmarkSystem,
    #[serde(default)]
    pub toolchains: Vec<ToolchainEvidence>,
    pub runtimes: Vec<RuntimeEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolchainEvidence {
    pub id: String,
    pub expected_version: String,
    pub actual_version: String,
    pub executable: String,
    pub executable_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub id: String,
    pub expected_version: String,
    pub actual_version: Option<String>,
    pub build_mode: String,
    pub executable: Option<String>,
    pub executable_sha256: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub execution_order: usize,
    pub startup_ms: Option<Distribution>,
    pub idle_memory: Option<MemoryMeasurement>,
    pub workloads: Vec<HttpWorkloadEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Distribution {
    pub samples: Vec<f64>,
    pub p50: f64,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub p50_ci95: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryMeasurement {
    pub value_kb: u64,
    pub kind: String,
    pub process_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpWorkloadEvidence {
    pub id: String,
    pub path: String,
    pub concurrency: usize,
    pub round_duration_ms: u64,
    pub rounds: Vec<HttpRound>,
    pub requests_per_second: Distribution,
    pub latency_ms: Distribution,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpRound {
    pub duration_ms: f64,
    pub requests: usize,
    pub requests_per_second: f64,
    pub latency_ms: Vec<f64>,
    pub errors: usize,
    #[serde(default)]
    pub server_cpu_core_pct: Option<f64>,
    #[serde(default)]
    pub client_cpu_core_pct: Option<f64>,
    #[serde(default)]
    pub peak_memory_kb: Option<u64>,
    #[serde(default)]
    pub memory_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComparisonSummary {
    pub schema_version: u32,
    pub generated_at_unix_ms: u128,
    pub source_commit: String,
    pub system: BenchmarkSystem,
    pub matrix: String,
    pub runtime_lock: String,
    pub quick: bool,
    pub order_seeds: Vec<u64>,
    pub inputs: Vec<SummaryInput>,
    #[serde(default)]
    pub toolchains: Vec<ToolchainEvidence>,
    pub runtimes: Vec<RuntimeSummary>,
    pub baseline: Option<BaselineComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SummaryInput {
    pub path: String,
    pub run_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSummary {
    pub id: String,
    pub actual_version: String,
    pub build_mode: String,
    pub executable_sha256: String,
    pub startup_ms: Distribution,
    pub idle_memory_kb: Distribution,
    pub memory_kind: String,
    pub workloads: Vec<HttpWorkloadSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpWorkloadSummary {
    pub id: String,
    pub path: String,
    pub concurrency: usize,
    pub rounds: usize,
    pub round_duration_ms: u64,
    pub requests_per_second: Distribution,
    pub latency_ms: Distribution,
    pub errors: usize,
    pub server_cpu_core_pct: Option<Distribution>,
    pub client_cpu_core_pct: Option<Distribution>,
    pub requests_per_server_cpu_second: Option<Distribution>,
    pub peak_memory_kb: Option<Distribution>,
    pub memory_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaselineComparison {
    pub source_commit: String,
    pub threshold_pct: f64,
    pub regressions: usize,
    pub improvements: usize,
    pub equivalent: usize,
    pub changes: Vec<MetricChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricChange {
    pub runtime: String,
    pub metric: String,
    pub workload: Option<String>,
    pub concurrency: Option<usize>,
    pub baseline: f64,
    pub current: f64,
    /// Positive means improvement; negative means regression.
    pub improvement_pct: f64,
    pub classification: String,
}

pub fn load_matrix(path: &Path) -> Result<Matrix> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let matrix: Matrix =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    validate_matrix(&matrix)?;
    Ok(matrix)
}

pub fn load_runtime_lock(path: &Path) -> Result<RuntimeLock> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let lock: RuntimeLock =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    ensure!(lock.schema_version == 1, "runtime lock schemaVersion must be 1");
    let mut ids = BTreeSet::new();
    for runtime in &lock.runtimes {
        ensure!(ids.insert(runtime.id.as_str()), "duplicate runtime id {}", runtime.id);
        ensure!(
            !runtime.expected_version.is_empty(),
            "runtime {} has no expectedVersion",
            runtime.id
        );
        ensure!(
            !runtime.readiness_prefix.is_empty(),
            "runtime {} has no readinessPrefix",
            runtime.id
        );
    }
    for required in ["tysel", "node", "bun", "deno"] {
        ensure!(ids.contains(required), "runtime lock is missing {required}");
    }
    Ok(lock)
}

pub fn validate_matrix(matrix: &Matrix) -> Result<()> {
    ensure!(matrix.schema_version == 1, "matrix schema_version must be 1");
    ensure!(matrix.measurement.startup_samples > 0, "startup_samples must be positive");
    ensure!(matrix.measurement.http_rounds > 0, "http_rounds must be positive");
    ensure!(
        matrix.measurement.http_round_duration_ms > 0,
        "http_round_duration_ms must be positive"
    );
    ensure!(!matrix.measurement.concurrency.is_empty(), "concurrency must not be empty");
    ensure!(
        matrix.measurement.concurrency.iter().all(|value| *value > 0),
        "concurrency must be positive"
    );
    ensure!(!matrix.workloads.is_empty(), "workloads must not be empty");
    let mut ids = BTreeSet::new();
    for workload in &matrix.workloads {
        ensure!(ids.insert(workload.id.as_str()), "duplicate workload id {}", workload.id);
        ensure!(workload.path.starts_with('/'), "workload {} path must start with /", workload.id);
        expected_body(&workload.response)?;
    }
    Ok(())
}

pub fn quick_matrix(mut matrix: Matrix) -> Matrix {
    matrix.measurement.startup_warmups = 1;
    matrix.measurement.startup_samples = 3;
    matrix.measurement.idle_settle_ms = 100;
    matrix.measurement.http_warmup_requests = 2;
    matrix.measurement.http_rounds = 2;
    matrix.measurement.http_round_duration_ms = 100;
    matrix.measurement.concurrency = vec![1, 4];
    matrix
}

pub fn expected_body(kind: &str) -> Result<Vec<u8>> {
    let body = match kind {
        "health" => b"ok".to_vec(),
        "json-1k" => serde_json::to_vec(&serde_json::json!({"payload": "a".repeat(1024)}))?,
        "json-64k" => serde_json::to_vec(&serde_json::json!({"payload": "b".repeat(65_536)}))?,
        "bytes-64k" => vec![b'x'; 65_536],
        other => anyhow::bail!("unsupported response kind {other}"),
    };
    Ok(body)
}

pub fn distribution(samples: Vec<f64>, bootstrap_seed: u64) -> Distribution {
    let p50 = percentile(&samples, 0.50);
    let p95 = (samples.len() >= 20).then(|| percentile(&samples, 0.95));
    let p99 = (samples.len() >= 100).then(|| percentile(&samples, 0.99));
    let p50_ci95 = (samples.len() >= 5).then(|| bootstrap_median_ci95(&samples, bootstrap_seed));
    Distribution { samples, p50, p95, p99, p50_ci95 }
}

fn distribution_with_ci_samples(
    samples: Vec<f64>,
    ci_samples: &[f64],
    bootstrap_seed: u64,
) -> Distribution {
    let p50 = percentile(&samples, 0.50);
    let p95 = (samples.len() >= 20).then(|| percentile(&samples, 0.95));
    let p99 = (samples.len() >= 100).then(|| percentile(&samples, 0.99));
    let p50_ci95 =
        (ci_samples.len() >= 5).then(|| bootstrap_median_ci95(ci_samples, bootstrap_seed));
    Distribution { samples, p50, p95, p99, p50_ci95 }
}

pub fn bootstrap_median_ci95(samples: &[f64], seed: u64) -> [f64; 2] {
    if samples.is_empty() {
        return [0.0, 0.0];
    }
    let mut state = seed.max(1);
    let mut medians = Vec::with_capacity(2_000);
    let mut resample = Vec::with_capacity(samples.len());
    for _ in 0..2_000 {
        resample.clear();
        for _ in 0..samples.len() {
            state = xorshift64(state);
            resample.push(samples[(state as usize) % samples.len()]);
        }
        medians.push(percentile(&resample, 0.50));
    }
    [percentile(&medians, 0.025), percentile(&medians, 0.975)]
}

fn xorshift64(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    value
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {} for SHA-256", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn resolve_executable(root: &Path, value: &str) -> Option<PathBuf> {
    let candidate = Path::new(value);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        let path =
            if candidate.is_absolute() { candidate.to_owned() } else { root.join(candidate) };
        return path.is_file().then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        let path = dir.join(value);
        path.is_file().then_some(path)
    })
}

pub fn git_state(root: &Path) -> Result<(String, bool)> {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .context("git rev-parse HEAD")?;
    ensure!(commit.status.success(), "git rev-parse HEAD failed");
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(root)
        .output()
        .context("git status")?;
    ensure!(status.status.success(), "git status failed");
    Ok((String::from_utf8(commit.stdout)?.trim().to_owned(), !status.stdout.is_empty()))
}

pub fn now_unix_ms() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

pub fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

pub fn benchmark_system() -> BenchmarkSystem {
    BenchmarkSystem {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        os_version: os_version(),
        cpu_model: cpu_model(),
    }
}

fn os_version() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(text) = fs::read_to_string("/etc/os-release") {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                return value.trim_matches('"').to_owned();
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(output) = Command::new("sw_vers").arg("-productVersion").output()
        && output.status.success()
    {
        return format!("macOS {}", String::from_utf8_lossy(&output.stdout).trim());
    }
    std::env::consts::OS.into()
}

fn cpu_model() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(text) = fs::read_to_string("/proc/cpuinfo") {
        for key in ["model name", "Hardware", "Processor"] {
            if let Some(value) = text.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == key).then(|| value.trim().to_owned())
            }) {
                return value;
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(output) = Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !value.is_empty() {
            return value;
        }
    }
    std::env::consts::ARCH.into()
}

pub fn process_memory_kb(pid: u32) -> Result<(u64, &'static str)> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/smaps_rollup");
        let text = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
        let kb = text
            .lines()
            .find_map(|line| {
                let (key, rest) = line.split_once(':')?;
                (key == "Pss").then(|| rest.split_whitespace().next()?.parse().ok())?
            })
            .ok_or_else(|| anyhow::anyhow!("Pss: missing from {path}"))?;
        Ok((kb, "pss"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output()?;
        ensure!(output.status.success(), "ps failed");
        let kb = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>()?;
        Ok((kb, "rss"))
    }
}

pub fn percentile(samples: &[f64], quantile: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * quantile).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

pub fn render_markdown(evidence: &ComparisonEvidence) -> String {
    let mut out = format!(
        "# Tysel runtime comparison\n\nRun `{}` on `{}` / `{}`. This is an internal engineering snapshot; it is not an architecture-aggregated score.\n\n## Source toolchain\n\n| Toolchain | Expected | Actual | Executable SHA-256 |\n| --- | --- | --- | --- |\n",
        evidence.run_id, evidence.system.os, evidence.system.arch
    );
    for toolchain in &evidence.toolchains {
        out.push_str(&format!(
            "| {} | {} | {} | `{}` |\n",
            toolchain.id,
            toolchain.expected_version,
            toolchain.actual_version,
            toolchain.executable_sha256
        ));
    }
    out.push_str("\nAll adapters are checked as TypeScript 7 source before runtime measurement. Type-check and build time are excluded from startup and HTTP metrics.\n\n## Runtime readiness\n\n| Runtime | Status | Version | Startup p50 | Idle memory |\n| --- | --- | --- | ---: | ---: |\n");
    for runtime in &evidence.runtimes {
        let startup = runtime
            .startup_ms
            .as_ref()
            .map(|value| format!("{:.2} ms", value.p50))
            .unwrap_or_else(|| "—".into());
        let memory = runtime
            .idle_memory
            .as_ref()
            .map(|value| format!("{:.2} MiB {}", value.value_kb as f64 / 1024.0, value.kind))
            .unwrap_or_else(|| "—".into());
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            runtime.id,
            runtime.status,
            runtime.actual_version.as_deref().unwrap_or("—"),
            startup,
            memory
        ));
    }
    out.push_str("\n## HTTP results\n\n| Runtime | Workload | Concurrency | Median req/s | Latency p50 | Errors |\n| --- | --- | ---: | ---: | ---: | ---: |\n");
    for runtime in &evidence.runtimes {
        for workload in &runtime.workloads {
            out.push_str(&format!(
                "| {} | {} | {} | {:.1} | {:.3} ms | {} |\n",
                runtime.id,
                workload.id,
                workload.concurrency,
                workload.requests_per_second.p50,
                workload.latency_ms.p50,
                workload.errors
            ));
        }
    }
    out.push_str("\n## Interpretation rules\n\n- Compare results only within the same architecture and run environment.\n- Treat a difference inside ±5% as practically equivalent until repeated evidence says otherwise.\n- Missing, failed, or semantically unequal cases are not converted to zero.\n- Peer results are observational; Tysel release gates remain separate.\n");
    out
}

pub fn aggregate_comparisons(
    inputs: Vec<(SummaryInput, ComparisonEvidence)>,
    required_runs: usize,
    allow_quick: bool,
    allow_dirty: bool,
    generated_at_unix_ms: u128,
) -> Result<ComparisonSummary> {
    ensure!(required_runs > 0, "required_runs must be positive");
    ensure!(
        inputs.len() == required_runs,
        "expected {required_runs} evidence files, got {}",
        inputs.len()
    );
    let (_, first) = inputs.first().context("no comparison evidence supplied")?;
    ensure!(
        first.schema_version == COMPARISON_SCHEMA_VERSION,
        "unsupported comparison schemaVersion {}",
        first.schema_version
    );
    ensure!(allow_quick || !first.quick, "quick evidence cannot produce a record summary");
    ensure!(
        allow_dirty || !first.workspace_dirty,
        "dirty evidence cannot produce a record summary"
    );
    ensure!(!first.toolchains.is_empty(), "record evidence has no source toolchain");

    let expected_runtime_ids = BTreeSet::from(["bun", "deno", "node", "tysel"]);
    let mut order_seeds = BTreeSet::new();
    let mut order_signatures = BTreeSet::new();
    for (_, evidence) in &inputs {
        ensure!(
            evidence.schema_version == first.schema_version,
            "comparison schemaVersion differs across inputs"
        );
        ensure!(
            evidence.source_commit == first.source_commit,
            "source commit differs across inputs"
        );
        ensure!(evidence.system == first.system, "system fingerprint differs across inputs");
        ensure!(evidence.matrix == first.matrix, "matrix differs across inputs");
        ensure!(evidence.runtime_lock == first.runtime_lock, "runtime lock differs across inputs");
        ensure!(
            toolchains_match(&evidence.toolchains, &first.toolchains),
            "source toolchain differs across inputs"
        );
        ensure!(evidence.quick == first.quick, "quick mode differs across inputs");
        ensure!(allow_quick || !evidence.quick, "quick evidence cannot produce a record summary");
        ensure!(
            allow_dirty || !evidence.workspace_dirty,
            "dirty evidence cannot produce a record summary"
        );
        ensure!(
            order_seeds.insert(evidence.order_seed),
            "duplicate order seed {}",
            evidence.order_seed
        );
        let ids: BTreeSet<_> =
            evidence.runtimes.iter().map(|runtime| runtime.id.as_str()).collect();
        ensure!(
            ids == expected_runtime_ids,
            "record evidence must contain tysel, node, bun, and deno"
        );
        ensure!(
            evidence.runtimes.iter().all(|runtime| runtime.status == "measured"),
            "record evidence contains an unavailable runtime"
        );
        let mut ordered: Vec<_> = evidence
            .runtimes
            .iter()
            .map(|runtime| (runtime.execution_order, runtime.id.as_str()))
            .collect();
        ordered.sort_unstable();
        let signature = ordered.into_iter().map(|(_, id)| id).collect::<Vec<_>>().join(",");
        ensure!(order_signatures.insert(signature), "runtime execution order was not rotated");
    }

    let mut runtimes = Vec::new();
    for id in expected_runtime_ids {
        let first_runtime = find_runtime(first, id)?;
        let actual_version =
            required_text(first_runtime.actual_version.as_deref(), id, "actualVersion")?;
        let executable_sha256 =
            required_text(first_runtime.executable_sha256.as_deref(), id, "executableSha256")?;
        let build_mode = first_runtime.build_mode.clone();
        let first_memory = first_runtime
            .idle_memory
            .as_ref()
            .with_context(|| format!("runtime {id} has no idle memory sample"))?;
        let mut startup_samples = Vec::new();
        let mut memory_samples = Vec::new();
        let mut workload_samples: BTreeMap<(String, usize), WorkloadAccumulator> = BTreeMap::new();

        for (_, evidence) in &inputs {
            let runtime = find_runtime(evidence, id)?;
            ensure!(
                runtime.actual_version.as_deref() == Some(actual_version.as_str()),
                "runtime {id} version differs across inputs"
            );
            ensure!(
                runtime.build_mode == build_mode,
                "runtime {id} build mode differs across inputs"
            );
            ensure!(
                runtime.executable_sha256.as_deref() == Some(executable_sha256.as_str()),
                "runtime {id} executable hash differs across inputs"
            );
            let startup = runtime
                .startup_ms
                .as_ref()
                .with_context(|| format!("runtime {id} has no startup samples"))?;
            ensure!(!startup.samples.is_empty(), "runtime {id} has empty startup samples");
            startup_samples.extend_from_slice(&startup.samples);
            let memory = runtime
                .idle_memory
                .as_ref()
                .with_context(|| format!("runtime {id} has no idle memory sample"))?;
            ensure!(
                memory.kind == first_memory.kind,
                "runtime {id} memory kind differs across inputs"
            );
            memory_samples.push(memory.value_kb as f64);

            let workload_keys: BTreeSet<_> = runtime
                .workloads
                .iter()
                .map(|workload| (workload.id.as_str(), workload.concurrency))
                .collect();
            let first_keys: BTreeSet<_> = first_runtime
                .workloads
                .iter()
                .map(|workload| (workload.id.as_str(), workload.concurrency))
                .collect();
            ensure!(
                workload_keys == first_keys,
                "runtime {id} workload matrix differs across inputs"
            );
            for workload in &runtime.workloads {
                ensure!(workload.errors == 0, "runtime {id} workload {} has errors", workload.id);
                let key = (workload.id.clone(), workload.concurrency);
                let accumulator =
                    workload_samples.entry(key).or_insert_with(|| WorkloadAccumulator {
                        path: workload.path.clone(),
                        round_duration_ms: workload.round_duration_ms,
                        rounds: 0,
                        rps: Vec::new(),
                        latency: Vec::new(),
                        round_latency_p50: Vec::new(),
                        errors: 0,
                        server_cpu: Vec::new(),
                        client_cpu: Vec::new(),
                        peak_memory: Vec::new(),
                        memory_kind: None,
                    });
                ensure!(accumulator.path == workload.path, "runtime {id} workload path differs");
                ensure!(
                    accumulator.round_duration_ms == workload.round_duration_ms,
                    "runtime {id} round duration differs"
                );
                accumulator.rounds += workload.rounds.len();
                let round_rps: Vec<_> =
                    workload.rounds.iter().map(|round| round.requests_per_second).collect();
                let round_latency: Vec<_> = workload
                    .rounds
                    .iter()
                    .flat_map(|round| round.latency_ms.iter().copied())
                    .collect();
                ensure!(
                    round_rps == workload.requests_per_second.samples,
                    "runtime {id} workload {} throughput samples disagree with rounds",
                    workload.id
                );
                ensure!(
                    round_latency == workload.latency_ms.samples,
                    "runtime {id} workload {} latency samples disagree with rounds",
                    workload.id
                );
                accumulator.rps.extend(round_rps);
                accumulator.latency.extend(round_latency);
                for round in &workload.rounds {
                    accumulator.round_latency_p50.push(percentile(&round.latency_ms, 0.50));
                    if let Some(value) = round.server_cpu_core_pct {
                        accumulator.server_cpu.push(value);
                    }
                    if let Some(value) = round.client_cpu_core_pct {
                        accumulator.client_cpu.push(value);
                    }
                    if let Some(value) = round.peak_memory_kb {
                        accumulator.peak_memory.push(value as f64);
                    }
                    if let Some(kind) = &round.memory_kind {
                        if let Some(expected) = &accumulator.memory_kind {
                            ensure!(expected == kind, "runtime {id} load memory kind differs");
                        } else {
                            accumulator.memory_kind = Some(kind.clone());
                        }
                    }
                }
                accumulator.errors += workload.errors;
            }
        }

        if first.system.os == "linux" {
            for ((workload_id, concurrency), samples) in &workload_samples {
                ensure!(
                    samples.server_cpu.len() == samples.rounds,
                    "runtime {id} workload {workload_id}/{concurrency} lacks server CPU samples"
                );
                ensure!(
                    samples.client_cpu.len() == samples.rounds,
                    "runtime {id} workload {workload_id}/{concurrency} lacks client CPU samples"
                );
                ensure!(
                    samples.peak_memory.len() == samples.rounds,
                    "runtime {id} workload {workload_id}/{concurrency} lacks peak memory samples"
                );
                ensure!(
                    samples.memory_kind.is_some(),
                    "runtime {id} workload {workload_id}/{concurrency} lacks memory kind"
                );
            }
        }
        let workloads = workload_samples
            .into_iter()
            .map(|((workload_id, concurrency), samples)| {
                let cpu_efficiency: Vec<_> = samples
                    .rps
                    .iter()
                    .zip(&samples.server_cpu)
                    .filter(|(_, cpu)| **cpu > 0.0)
                    .map(|(rps, cpu)| rps / (cpu / 100.0))
                    .collect();
                HttpWorkloadSummary {
                    id: workload_id.clone(),
                    path: samples.path,
                    concurrency,
                    rounds: samples.rounds,
                    round_duration_ms: samples.round_duration_ms,
                    requests_per_second: distribution(
                        samples.rps,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/rps")),
                    ),
                    latency_ms: distribution_with_ci_samples(
                        samples.latency,
                        &samples.round_latency_p50,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/latency")),
                    ),
                    errors: samples.errors,
                    server_cpu_core_pct: optional_distribution(
                        samples.server_cpu,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/server-cpu")),
                    ),
                    client_cpu_core_pct: optional_distribution(
                        samples.client_cpu,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/client-cpu")),
                    ),
                    requests_per_server_cpu_second: optional_distribution(
                        cpu_efficiency,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/cpu-efficiency")),
                    ),
                    peak_memory_kb: optional_distribution(
                        samples.peak_memory,
                        stable_seed(&format!("{id}/{workload_id}/{concurrency}/peak-memory")),
                    ),
                    memory_kind: samples.memory_kind,
                }
            })
            .collect();
        runtimes.push(RuntimeSummary {
            id: id.to_owned(),
            actual_version,
            build_mode,
            executable_sha256,
            startup_ms: distribution(startup_samples, stable_seed(&format!("{id}/startup"))),
            idle_memory_kb: distribution(memory_samples, stable_seed(&format!("{id}/memory"))),
            memory_kind: first_memory.kind.clone(),
            workloads,
        });
    }

    let toolchains = first.toolchains.clone();
    Ok(ComparisonSummary {
        schema_version: COMPARISON_SUMMARY_SCHEMA_VERSION,
        generated_at_unix_ms,
        source_commit: first.source_commit.clone(),
        system: first.system.clone(),
        matrix: first.matrix.clone(),
        runtime_lock: first.runtime_lock.clone(),
        quick: first.quick,
        order_seeds: order_seeds.into_iter().collect(),
        inputs: inputs.into_iter().map(|(input, _)| input).collect(),
        toolchains,
        runtimes,
        baseline: None,
    })
}

pub fn compare_to_baseline(
    current: &ComparisonSummary,
    baseline: &ComparisonSummary,
    threshold_pct: f64,
) -> Result<BaselineComparison> {
    ensure!(threshold_pct >= 0.0, "regression threshold must not be negative");
    ensure!(current.system == baseline.system, "baseline system fingerprint differs");
    ensure!(current.matrix == baseline.matrix, "baseline matrix differs");
    ensure!(current.runtime_lock == baseline.runtime_lock, "baseline runtime lock differs");
    ensure!(
        toolchains_match(&current.toolchains, &baseline.toolchains),
        "baseline source toolchain differs"
    );
    ensure!(!current.quick && !baseline.quick, "quick summaries cannot be regression baselines");
    let mut changes = Vec::new();
    for runtime in &current.runtimes {
        let previous = baseline
            .runtimes
            .iter()
            .find(|candidate| candidate.id == runtime.id)
            .with_context(|| format!("baseline is missing runtime {}", runtime.id))?;
        push_change(
            &mut changes,
            &runtime.id,
            "startup-p50-ms",
            None,
            None,
            previous.startup_ms.p50,
            runtime.startup_ms.p50,
            false,
            threshold_pct,
        );
        push_change(
            &mut changes,
            &runtime.id,
            "idle-memory-p50-kb",
            None,
            None,
            previous.idle_memory_kb.p50,
            runtime.idle_memory_kb.p50,
            false,
            threshold_pct,
        );
        for workload in &runtime.workloads {
            let prior_workload = previous
                .workloads
                .iter()
                .find(|candidate| {
                    candidate.id == workload.id && candidate.concurrency == workload.concurrency
                })
                .with_context(|| {
                    format!(
                        "baseline is missing runtime {} workload {} concurrency {}",
                        runtime.id, workload.id, workload.concurrency
                    )
                })?;
            push_change(
                &mut changes,
                &runtime.id,
                "requests-per-second-p50",
                Some(&workload.id),
                Some(workload.concurrency),
                prior_workload.requests_per_second.p50,
                workload.requests_per_second.p50,
                true,
                threshold_pct,
            );
            push_change(
                &mut changes,
                &runtime.id,
                "latency-p50-ms",
                Some(&workload.id),
                Some(workload.concurrency),
                prior_workload.latency_ms.p50,
                workload.latency_ms.p50,
                false,
                threshold_pct,
            );
            push_optional_change(
                &mut changes,
                &runtime.id,
                "requests-per-server-cpu-second-p50",
                &workload.id,
                workload.concurrency,
                prior_workload.requests_per_server_cpu_second.as_ref(),
                workload.requests_per_server_cpu_second.as_ref(),
                true,
                threshold_pct,
            )?;
            push_optional_change(
                &mut changes,
                &runtime.id,
                "peak-memory-p50-kb",
                &workload.id,
                workload.concurrency,
                prior_workload.peak_memory_kb.as_ref(),
                workload.peak_memory_kb.as_ref(),
                false,
                threshold_pct,
            )?;
        }
    }
    let regressions = changes.iter().filter(|change| change.classification == "regression").count();
    let improvements =
        changes.iter().filter(|change| change.classification == "improvement").count();
    let equivalent = changes.iter().filter(|change| change.classification == "equivalent").count();
    Ok(BaselineComparison {
        source_commit: baseline.source_commit.clone(),
        threshold_pct,
        regressions,
        improvements,
        equivalent,
        changes,
    })
}

fn toolchains_match(left: &[ToolchainEvidence], right: &[ToolchainEvidence]) -> bool {
    left.len() == right.len()
        && left.iter().all(|toolchain| {
            right.iter().any(|candidate| {
                candidate.id == toolchain.id
                    && candidate.expected_version == toolchain.expected_version
                    && candidate.actual_version == toolchain.actual_version
                    && candidate.executable_sha256 == toolchain.executable_sha256
            })
        })
}

pub fn render_summary_markdown(summary: &ComparisonSummary) -> String {
    let peer_rows = peer_comparisons(summary);
    let leads = peer_rows.iter().filter(|row| row.classification == "lead").count();
    let equivalent = peer_rows.iter().filter(|row| row.classification == "equivalent").count();
    let trails = peer_rows.iter().filter(|row| row.classification == "trail").count();
    let mut out = format!(
        "# Tysel cross-runtime performance report\n\n## Technical summary\n\nThis report aggregates {} rotated runs on `{}` / `{}` for commit `{}`. Against the strongest measured peer for each headline metric, Tysel leads on {}, is within ±5% on {}, and trails on {}. These are descriptive comparisons, not claims of statistical superiority. Results are comparable only within this system fingerprint and locked runtime matrix.\n\n## Tysel's position against the strongest peer\n\n| Metric | Workload | Concurrency | Strongest peer | Tysel | Peer | Tysel advantage | Position |\n| --- | --- | ---: | --- | ---: | ---: | ---: | --- |\n",
        summary.inputs.len(),
        summary.system.os,
        summary.system.arch,
        summary.source_commit,
        leads,
        equivalent,
        trails
    );
    for row in &peer_rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:.3} | {:.3} | {:+.2}% | {} |\n",
            row.metric,
            row.workload.as_deref().unwrap_or("—"),
            row.concurrency.map_or_else(|| "—".to_owned(), |value| value.to_string()),
            row.peer,
            row.tysel,
            row.peer_value,
            row.advantage_pct,
            row.classification
        ));
    }
    out.push_str("\n## Source toolchain\n\n| Toolchain | Expected | Actual | Executable SHA-256 |\n| --- | --- | --- | --- |\n");
    for toolchain in &summary.toolchains {
        out.push_str(&format!(
            "| {} | {} | {} | `{}` |\n",
            toolchain.id,
            toolchain.expected_version,
            toolchain.actual_version,
            toolchain.executable_sha256
        ));
    }
    out.push_str("\n## Runtime-level startup and memory evidence\n\n| Runtime | Version | Startup p50 | Idle memory p50 |\n| --- | --- | ---: | ---: |\n");
    for runtime in &summary.runtimes {
        out.push_str(&format!(
            "| {} | {} | {:.2} ms | {:.2} MiB {} |\n",
            runtime.id,
            runtime.actual_version,
            runtime.startup_ms.p50,
            runtime.idle_memory_kb.p50 / 1024.0,
            runtime.memory_kind
        ));
    }
    out.push_str("\n## Matched HTTP workload evidence\n\n| Runtime | Workload | Concurrency | Median req/s | Latency p50 | Server CPU | Client CPU | Req/s per server core | Peak memory | Errors |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for runtime in &summary.runtimes {
        for workload in &runtime.workloads {
            out.push_str(&format!(
                "| {} | {} | {} | {:.1} | {:.3} ms | {} | {} | {} | {} | {} |\n",
                runtime.id,
                workload.id,
                workload.concurrency,
                workload.requests_per_second.p50,
                workload.latency_ms.p50,
                format_optional(workload.server_cpu_core_pct.as_ref(), "core%", 1.0),
                format_optional(workload.client_cpu_core_pct.as_ref(), "core%", 1.0),
                format_optional(
                    workload.requests_per_server_cpu_second.as_ref(),
                    "req/s/core",
                    1.0
                ),
                format_optional(workload.peak_memory_kb.as_ref(), "MiB", 1.0 / 1024.0),
                workload.errors
            ));
        }
    }
    if let Some(baseline) = &summary.baseline {
        out.push_str(&format!(
            "\n## Regression analysis\n\nAgainst commit `{}`, using a ±{:.1}% practical-equivalence threshold: {} improvements, {} equivalent metrics, and {} regressions. Positive change means improvement for every metric.\n\n| Runtime | Metric | Workload | Concurrency | Baseline | Current | Improvement | Classification |\n| --- | --- | --- | ---: | ---: | ---: | ---: | --- |\n",
            baseline.source_commit,
            baseline.threshold_pct,
            baseline.improvements,
            baseline.equivalent,
            baseline.regressions
        ));
        for change in &baseline.changes {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.3} | {:.3} | {:+.2}% | {} |\n",
                change.runtime,
                change.metric,
                change.workload.as_deref().unwrap_or("—"),
                change.concurrency.map_or_else(|| "—".to_owned(), |value| value.to_string()),
                change.baseline,
                change.current,
                change.improvement_pct,
                change.classification
            ));
        }
    }
    out.push_str("\n## Scope, data, and metric definitions\n\n- Startup is process spawn to readiness announcement; lower is better.\n- Idle memory is the Linux process-tree memory value reported by the evidence; lower is better.\n- Throughput is completed requests per second; higher is better. Latency is client-observed; lower is better.\n- Each row pools raw samples across rotated runtime orders; no cross-architecture ranking is calculated.\n\n## Measurement methodology\n\nEvery adapter is strict-checked as TypeScript 7 source before measurement, then executed through its runtime's native TypeScript path. Type-check and build time are outside startup and HTTP timing. Every runtime serves byte-identical response contracts. Four order seeds rotate the runtime sequence only after all preparation is complete. The summary retains hashes of every input evidence file and pools raw startup, round-throughput, and request-latency samples. Central latency percentiles use every request; latency-median confidence intervals bootstrap the 40 independent round medians instead of treating correlated requests within a round as independent observations.\n\n## Limitations, uncertainty, and robustness checks\n\nThe ±5% band is a practical-equivalence rule, not a statistical significance test. TypeScript 7 validates shared source semantics but each runtime still owns parsing, lowering, and execution. The in-process load generator must also be checked for client saturation on each fixed runner. A valid summary rejects dirty workspaces, environment mismatches, source-toolchain changes, missing runtimes, duplicate execution orders, changed executable hashes, and any HTTP error.\n\n## Recommended next steps\n\n1. Use the regression table to gate Tysel-only changes against the previous summary on the same fixed host.\n2. Investigate the largest trailing workload before optimizing aggregate scores.\n3. Collect three stable record cycles on both architectures before selecting website claims.\n\n## Further questions\n\n- Does an external load-generator host reproduce the same concurrency ranking?\n- Are CPU utilization, loaded memory, HTTP/2, streaming, SSE, and WebSocket results consistent with the HTTP/1.1 track?\n- Which Tysel subsystem explains each material regression or advantage?\n");
    out
}

#[derive(Debug)]
struct PeerComparisonRow {
    metric: String,
    workload: Option<String>,
    concurrency: Option<usize>,
    peer: String,
    tysel: f64,
    peer_value: f64,
    advantage_pct: f64,
    classification: String,
}

fn peer_comparisons(summary: &ComparisonSummary) -> Vec<PeerComparisonRow> {
    let Some(tysel) = summary.runtimes.iter().find(|runtime| runtime.id == "tysel") else {
        return Vec::new();
    };
    let peers: Vec<_> = summary.runtimes.iter().filter(|runtime| runtime.id != "tysel").collect();
    let mut rows = Vec::new();
    if let Some(peer) =
        peers.iter().min_by(|left, right| left.startup_ms.p50.total_cmp(&right.startup_ms.p50))
    {
        rows.push(peer_row(
            "startup-p50-ms",
            None,
            None,
            &peer.id,
            tysel.startup_ms.p50,
            peer.startup_ms.p50,
            false,
        ));
    }
    if let Some(peer) = peers
        .iter()
        .min_by(|left, right| left.idle_memory_kb.p50.total_cmp(&right.idle_memory_kb.p50))
    {
        rows.push(peer_row(
            "idle-memory-p50-kb",
            None,
            None,
            &peer.id,
            tysel.idle_memory_kb.p50,
            peer.idle_memory_kb.p50,
            false,
        ));
    }
    for workload in &tysel.workloads {
        let candidates: Vec<_> = peers
            .iter()
            .filter_map(|runtime| {
                runtime
                    .workloads
                    .iter()
                    .find(|candidate| {
                        candidate.id == workload.id && candidate.concurrency == workload.concurrency
                    })
                    .map(|candidate| (*runtime, candidate))
            })
            .collect();
        if let Some((peer, candidate)) = candidates.iter().max_by(|left, right| {
            left.1.requests_per_second.p50.total_cmp(&right.1.requests_per_second.p50)
        }) {
            rows.push(peer_row(
                "requests-per-second-p50",
                Some(&workload.id),
                Some(workload.concurrency),
                &peer.id,
                workload.requests_per_second.p50,
                candidate.requests_per_second.p50,
                true,
            ));
        }
        if let Some((peer, candidate)) = candidates
            .iter()
            .min_by(|left, right| left.1.latency_ms.p50.total_cmp(&right.1.latency_ms.p50))
        {
            rows.push(peer_row(
                "latency-p50-ms",
                Some(&workload.id),
                Some(workload.concurrency),
                &peer.id,
                workload.latency_ms.p50,
                candidate.latency_ms.p50,
                false,
            ));
        }
        if let Some(tysel_efficiency) = &workload.requests_per_server_cpu_second
            && let Some((peer, candidate)) = candidates
                .iter()
                .filter_map(|(runtime, candidate)| {
                    candidate.requests_per_server_cpu_second.as_ref().map(|value| (*runtime, value))
                })
                .max_by(|left, right| left.1.p50.total_cmp(&right.1.p50))
        {
            rows.push(peer_row(
                "requests-per-server-cpu-second-p50",
                Some(&workload.id),
                Some(workload.concurrency),
                &peer.id,
                tysel_efficiency.p50,
                candidate.p50,
                true,
            ));
        }
        if let Some(tysel_peak_memory) = &workload.peak_memory_kb
            && let Some((peer, candidate)) = candidates
                .iter()
                .filter_map(|(runtime, candidate)| {
                    candidate.peak_memory_kb.as_ref().map(|value| (*runtime, value))
                })
                .min_by(|left, right| left.1.p50.total_cmp(&right.1.p50))
        {
            rows.push(peer_row(
                "peak-memory-p50-kb",
                Some(&workload.id),
                Some(workload.concurrency),
                &peer.id,
                tysel_peak_memory.p50,
                candidate.p50,
                false,
            ));
        }
    }
    rows
}

fn peer_row(
    metric: &str,
    workload: Option<&str>,
    concurrency: Option<usize>,
    peer: &str,
    tysel: f64,
    peer_value: f64,
    higher_is_better: bool,
) -> PeerComparisonRow {
    let raw_pct = relative_change_pct(tysel, peer_value);
    let advantage_pct = if higher_is_better { raw_pct } else { -raw_pct };
    let classification = if advantage_pct > 5.0 {
        "lead"
    } else if advantage_pct < -5.0 {
        "trail"
    } else {
        "equivalent"
    };
    PeerComparisonRow {
        metric: metric.to_owned(),
        workload: workload.map(str::to_owned),
        concurrency,
        peer: peer.to_owned(),
        tysel,
        peer_value,
        advantage_pct,
        classification: classification.to_owned(),
    }
}

fn format_optional(distribution: Option<&Distribution>, unit: &str, scale: f64) -> String {
    distribution.map_or_else(|| "—".to_owned(), |value| format!("{:.1} {unit}", value.p50 * scale))
}

#[derive(Debug)]
struct WorkloadAccumulator {
    path: String,
    round_duration_ms: u64,
    rounds: usize,
    rps: Vec<f64>,
    latency: Vec<f64>,
    round_latency_p50: Vec<f64>,
    errors: usize,
    server_cpu: Vec<f64>,
    client_cpu: Vec<f64>,
    peak_memory: Vec<f64>,
    memory_kind: Option<String>,
}

fn optional_distribution(samples: Vec<f64>, seed: u64) -> Option<Distribution> {
    (!samples.is_empty()).then(|| distribution(samples, seed))
}

fn find_runtime<'a>(evidence: &'a ComparisonEvidence, id: &str) -> Result<&'a RuntimeEvidence> {
    evidence
        .runtimes
        .iter()
        .find(|runtime| runtime.id == id)
        .with_context(|| format!("evidence {} is missing runtime {id}", evidence.run_id))
}

fn required_text(value: Option<&str>, runtime: &str, field: &str) -> Result<String> {
    let value = value.with_context(|| format!("runtime {runtime} has no {field}"))?;
    ensure!(!value.is_empty(), "runtime {runtime} has empty {field}");
    Ok(value.to_owned())
}

fn stable_seed(label: &str) -> u64 {
    let digest = Sha256::digest(label.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().expect("SHA-256 prefix has eight bytes"))
}

fn relative_change_pct(current: f64, baseline: f64) -> f64 {
    if baseline == 0.0 {
        if current == 0.0 { 0.0 } else { 100.0 }
    } else {
        (current - baseline) / baseline * 100.0
    }
}

#[allow(clippy::too_many_arguments)]
fn push_change(
    changes: &mut Vec<MetricChange>,
    runtime: &str,
    metric: &str,
    workload: Option<&str>,
    concurrency: Option<usize>,
    baseline: f64,
    current: f64,
    higher_is_better: bool,
    threshold_pct: f64,
) {
    let raw_pct = relative_change_pct(current, baseline);
    let improvement_pct = if higher_is_better { raw_pct } else { -raw_pct };
    let classification = if improvement_pct > threshold_pct {
        "improvement"
    } else if improvement_pct < -threshold_pct {
        "regression"
    } else {
        "equivalent"
    };
    changes.push(MetricChange {
        runtime: runtime.to_owned(),
        metric: metric.to_owned(),
        workload: workload.map(str::to_owned),
        concurrency,
        baseline,
        current,
        improvement_pct,
        classification: classification.to_owned(),
    });
}

#[allow(clippy::too_many_arguments)]
fn push_optional_change(
    changes: &mut Vec<MetricChange>,
    runtime: &str,
    metric: &str,
    workload: &str,
    concurrency: usize,
    baseline: Option<&Distribution>,
    current: Option<&Distribution>,
    higher_is_better: bool,
    threshold_pct: f64,
) -> Result<()> {
    match (baseline, current) {
        (Some(baseline), Some(current)) => push_change(
            changes,
            runtime,
            metric,
            Some(workload),
            Some(concurrency),
            baseline.p50,
            current.p50,
            higher_is_better,
            threshold_pct,
        ),
        (None, None) => {}
        _ => anyhow::bail!(
            "baseline metric availability differs for runtime {runtime} workload {workload}/{concurrency} {metric}"
        ),
    }
    Ok(())
}

pub fn evidence_shell_command() -> String {
    std::env::args().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_evidence(seed: u64, commit: &str, dirty: bool, scale: f64) -> ComparisonEvidence {
        let mut ids = vec!["tysel", "node", "bun", "deno"];
        let length = ids.len();
        ids.rotate_left(seed as usize % length);
        let runtimes = ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| RuntimeEvidence {
                id: id.to_owned(),
                expected_version: "1.0.0".into(),
                actual_version: Some(format!("{id} 1.0.0")),
                build_mode: "runtime-source".into(),
                executable: Some(format!("/opt/{id}")),
                executable_sha256: Some(format!("{:064x}", stable_seed(id))),
                status: "measured".into(),
                reason: None,
                execution_order: index + 1,
                startup_ms: Some(distribution(vec![10.0 * scale; 5], seed)),
                idle_memory: Some(MemoryMeasurement {
                    value_kb: (1024.0 * scale) as u64,
                    kind: "pss".into(),
                    process_count: 1,
                }),
                workloads: vec![HttpWorkloadEvidence {
                    id: "health".into(),
                    path: "/health".into(),
                    concurrency: 1,
                    round_duration_ms: 500,
                    rounds: vec![HttpRound {
                        duration_ms: 10.0,
                        requests: 100,
                        requests_per_second: 1000.0 * scale,
                        latency_ms: vec![1.0 / scale; 100],
                        errors: 0,
                        server_cpu_core_pct: Some(75.0),
                        client_cpu_core_pct: Some(25.0),
                        peak_memory_kb: Some(2048),
                        memory_kind: Some("pss".into()),
                    }],
                    requests_per_second: distribution(vec![1000.0 * scale], seed),
                    latency_ms: distribution(vec![1.0 / scale; 100], seed),
                    errors: 0,
                }],
            })
            .collect();
        ComparisonEvidence {
            schema_version: COMPARISON_SCHEMA_VERSION,
            run_id: format!("run-{seed}"),
            generated_at_unix_ms: 1,
            source_commit: commit.into(),
            workspace_dirty: dirty,
            command: "compare".into(),
            matrix: "matrix.toml".into(),
            runtime_lock: "runtimes.lock.json".into(),
            quick: false,
            order_seed: seed,
            system: BenchmarkSystem {
                os: "linux".into(),
                arch: "x86_64".into(),
                os_version: "test".into(),
                cpu_model: "test cpu".into(),
            },
            toolchains: vec![ToolchainEvidence {
                id: "typescript".into(),
                expected_version: "7.0.2".into(),
                actual_version: "Version 7.0.2".into(),
                executable: "/opt/tsc".into(),
                executable_sha256: "e".repeat(64),
            }],
            runtimes,
        }
    }

    fn sample_inputs(
        commit: &str,
        dirty: bool,
        scale: f64,
    ) -> Vec<(SummaryInput, ComparisonEvidence)> {
        (1..=4)
            .map(|seed| {
                let evidence = sample_evidence(seed, commit, dirty, scale);
                (
                    SummaryInput {
                        path: format!("seed-{seed}.json"),
                        run_id: evidence.run_id.clone(),
                        sha256: format!("{seed:064x}"),
                    },
                    evidence,
                )
            })
            .collect()
    }

    #[test]
    fn response_fixtures_have_stable_sizes() {
        assert_eq!(expected_body("health").unwrap(), b"ok");
        assert_eq!(expected_body("bytes-64k").unwrap().len(), 65_536);
        assert_eq!(expected_body("json-1k").unwrap().len(), 1_038);
        assert_eq!(expected_body("json-64k").unwrap().len(), 65_550);
    }

    #[test]
    fn distributions_keep_raw_samples_and_tail_rules() {
        let value = distribution((0..100).map(f64::from).collect(), 7);
        assert_eq!(value.samples.len(), 100);
        assert!(value.p95.is_some());
        assert!(value.p99.is_some());
        assert!(value.p50_ci95.is_some());
    }

    #[test]
    fn quick_mode_is_explicitly_not_release_scale() {
        let matrix = Matrix {
            schema_version: 1,
            measurement: MeasurementConfig {
                startup_warmups: 10,
                startup_samples: 200,
                idle_settle_ms: 400,
                http_warmup_requests: 100,
                http_rounds: 10,
                http_round_duration_ms: 500,
                concurrency: vec![1, 10, 100],
                request_timeout_ms: 2_000,
            },
            workloads: vec![WorkloadConfig {
                id: "health".into(),
                path: "/health".into(),
                response: "health".into(),
            }],
        };
        let quick = quick_matrix(matrix);
        assert_eq!(quick.measurement.startup_samples, 3);
        assert_eq!(quick.measurement.http_rounds, 2);
        assert_eq!(quick.measurement.concurrency, [1, 4]);
    }

    #[test]
    fn runtime_lock_uses_typescript_7_sources() {
        let root = workspace_root();
        let lock =
            load_runtime_lock(&root.join("benchmarks/comparison/runtimes.lock.json")).unwrap();
        assert_eq!(lock.toolchains.len(), 1);
        assert_eq!(lock.toolchains[0].id, "typescript");
        assert_eq!(lock.toolchains[0].expected_version, "7.0.2");
        let node = lock.runtimes.iter().find(|runtime| runtime.id == "node").unwrap();
        assert_eq!(node.args, ["benchmarks/comparison/adapters/node/server.ts"]);
    }

    #[test]
    fn aggregate_requires_clean_rotated_full_runtime_evidence() {
        let summary =
            aggregate_comparisons(sample_inputs("a", false, 1.0), 4, false, false, 2).unwrap();
        assert_eq!(summary.order_seeds, [1, 2, 3, 4]);
        assert_eq!(summary.runtimes.len(), 4);
        let tysel = summary.runtimes.iter().find(|runtime| runtime.id == "tysel").unwrap();
        assert_eq!(tysel.startup_ms.samples.len(), 20);
        assert_eq!(tysel.idle_memory_kb.samples.len(), 4);
        assert_eq!(tysel.workloads[0].rounds, 4);
        assert_eq!(tysel.workloads[0].latency_ms.samples.len(), 400);
    }

    #[test]
    fn aggregate_rejects_dirty_record_evidence() {
        let error =
            aggregate_comparisons(sample_inputs("a", true, 1.0), 4, false, false, 2).unwrap_err();
        assert!(error.to_string().contains("dirty evidence"));
    }

    #[test]
    fn aggregate_rejects_mixed_source_toolchains() {
        let mut inputs = sample_inputs("a", false, 1.0);
        inputs[1].1.toolchains[0].actual_version = "Version 7.0.3".into();
        let error = aggregate_comparisons(inputs, 4, false, false, 2).unwrap_err();
        assert!(error.to_string().contains("source toolchain differs"));
    }

    #[test]
    fn baseline_change_direction_is_normalized_to_improvement() {
        let baseline =
            aggregate_comparisons(sample_inputs("a", false, 1.0), 4, false, false, 2).unwrap();
        let current =
            aggregate_comparisons(sample_inputs("b", false, 1.1), 4, false, false, 3).unwrap();
        let comparison = compare_to_baseline(&current, &baseline, 5.0).unwrap();
        let tysel_startup = comparison
            .changes
            .iter()
            .find(|change| change.runtime == "tysel" && change.metric == "startup-p50-ms")
            .unwrap();
        let tysel_throughput = comparison
            .changes
            .iter()
            .find(|change| change.runtime == "tysel" && change.metric == "requests-per-second-p50")
            .unwrap();
        assert_eq!(tysel_startup.classification, "regression");
        assert_eq!(tysel_throughput.classification, "improvement");
    }

    #[test]
    fn published_evidence_schemas_are_valid_json() {
        let comparison: serde_json::Value = serde_json::from_str(include_str!(
            "../../../benchmarks/comparison/schemas/comparison-v1.schema.json"
        ))
        .unwrap();
        let summary: serde_json::Value = serde_json::from_str(include_str!(
            "../../../benchmarks/comparison/schemas/comparison-summary-v1.schema.json"
        ))
        .unwrap();
        assert_eq!(comparison["properties"]["schemaVersion"]["const"], 1);
        assert_eq!(summary["properties"]["schemaVersion"]["const"], 1);
    }
}
