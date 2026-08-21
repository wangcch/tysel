//! Shared helpers and the M0 §30 measurement harness (`tysel-bench`).

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tysel_build::{embed, tap_from_app, transpile_typescript};
use tysel_manifest::Manifest;

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

pub const COLD_START_MS: f64 = 15.0;
pub const IDLE_MEMORY_MB: f64 = 32.0;
pub const ARTIFACT_MB: f64 = 20.0;
pub const BENCHMARK_EVIDENCE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct BenchReport {
    pub artifact_bytes: u64,
    pub artifact_sha256: String,
    pub cold_start_ms: Vec<f64>,
    pub idle_memory_kb: u64,
    pub memory_kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkEvidence {
    pub evidence_version: u32,
    pub source_commit: String,
    pub target: String,
    pub profile: String,
    pub command: String,
    pub system: BenchmarkSystem,
    pub artifact: BenchmarkArtifact,
    pub measurements: BenchmarkMeasurements,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSystem {
    pub os: String,
    pub arch: String,
    pub os_version: String,
    pub cpu_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkArtifact {
    pub kind: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkMeasurements {
    pub cold_start_ms: Vec<f64>,
    pub cold_start_p50_ms: BenchmarkGate,
    pub idle_memory_mb: BenchmarkGate,
    pub artifact_mb: BenchmarkGate,
    pub memory_kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkGate {
    pub measured: f64,
    pub limit: f64,
    pub passed: bool,
}

impl BenchReport {
    pub fn artifact_mb(&self) -> f64 {
        self.artifact_bytes as f64 / (1024.0 * 1024.0)
    }

    pub fn cold_start_p50_ms(&self) -> f64 {
        percentile(&self.cold_start_ms, 0.50)
    }

    pub fn idle_memory_mb(&self) -> f64 {
        self.idle_memory_kb as f64 / 1024.0
    }
}

pub fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

pub fn package_hello_service(stub: &Path, output: &Path) -> Result<()> {
    const MANIFEST: &str = include_str!("../../../examples/hello-service/tysel.toml");
    const SOURCE: &str = include_str!("../../../examples/hello-service/src/index.ts");

    let manifest = Manifest::parse(MANIFEST).context("embedded hello-service manifest")?;
    let virtual_entry = Path::new("benchmarks/hello-service/src/index.ts");
    let (bundle, source_map) =
        transpile_typescript(virtual_entry, SOURCE).context("transpile embedded hello-service")?;
    let mut tap = tap_from_app(&manifest, env!("CARGO_PKG_VERSION"), bundle, source_map);
    tap.manifest.listen = "127.0.0.1:0".into();
    embed(stub, output, &tap).context("embed TAP")?;
    Ok(())
}

pub fn measure(stub: &Path) -> Result<BenchReport> {
    let dir = std::env::temp_dir().join(format!("tysel-bench-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let packaged = dir.join("hello-service");
    package_hello_service(stub, &packaged)?;
    let artifact = fs::read(&packaged)?;
    let artifact_bytes = artifact.len() as u64;
    let artifact_sha256 = hex_sha256(&artifact);

    // Discard two runs so dyld / page cache are not the first sample.
    for index in 0..2 {
        let _ = timed_cold_start(&packaged)
            .with_context(|| format!("cold-start warm-up {} failed", index + 1))?;
    }
    let mut cold_start_ms = Vec::with_capacity(11);
    for index in 0..11 {
        cold_start_ms.push(
            timed_cold_start(&packaged)
                .with_context(|| format!("cold-start sample {} failed", index + 1))?
                .as_secs_f64()
                * 1_000.0,
        );
    }

    let mut child = spawn_service(&packaged)?;
    if let Err(error) = wait_listen(&mut child, Duration::from_secs(5)) {
        stop_child(&mut child);
        return Err(error).context("idle-memory service failed to start");
    }
    thread::sleep(Duration::from_millis(400));
    let pid = child.id();
    let (idle_memory_kb, memory_kind) = process_memory_kb(pid).context("sample idle memory")?;
    stop_child(&mut child);

    Ok(BenchReport { artifact_bytes, artifact_sha256, cold_start_ms, idle_memory_kb, memory_kind })
}

pub fn benchmark_evidence(
    report: &BenchReport,
    source_commit: &str,
    command: &str,
) -> Result<BenchmarkEvidence> {
    ensure!(
        valid_source_commit(source_commit),
        "source commit must be 40 or 64 lowercase hex characters"
    );
    ensure!(
        !command.is_empty() && command.len() <= 4096,
        "benchmark command must contain 1..=4096 bytes"
    );
    ensure!(!command.contains(['\r', '\n']), "benchmark command must be a single line");
    ensure!(
        report.cold_start_ms.len() == 11
            && report.cold_start_ms.iter().all(|sample| sample.is_finite() && *sample >= 0.0),
        "benchmark evidence requires 11 finite non-negative cold-start samples"
    );
    ensure!(valid_sha256(&report.artifact_sha256), "artifact digest must be lowercase SHA-256");

    let cold_start = report.cold_start_p50_ms();
    let memory = report.idle_memory_mb();
    let artifact = report.artifact_mb();
    Ok(BenchmarkEvidence {
        evidence_version: BENCHMARK_EVIDENCE_VERSION,
        source_commit: source_commit.to_owned(),
        target: benchmark_target(),
        profile: "release".into(),
        command: command.to_owned(),
        system: benchmark_system(),
        artifact: BenchmarkArtifact {
            kind: "tysel-single-executable".into(),
            size_bytes: report.artifact_bytes,
            sha256: report.artifact_sha256.clone(),
        },
        measurements: BenchmarkMeasurements {
            cold_start_ms: report.cold_start_ms.clone(),
            cold_start_p50_ms: BenchmarkGate {
                measured: cold_start,
                limit: COLD_START_MS,
                passed: cold_start <= COLD_START_MS,
            },
            idle_memory_mb: BenchmarkGate {
                measured: memory,
                limit: IDLE_MEMORY_MB,
                passed: memory <= IDLE_MEMORY_MB
                    && (!cfg!(target_os = "linux") || report.memory_kind == "pss"),
            },
            artifact_mb: BenchmarkGate {
                measured: artifact,
                limit: ARTIFACT_MB,
                passed: artifact <= ARTIFACT_MB,
            },
            memory_kind: report.memory_kind.into(),
        },
    })
}

pub fn write_benchmark_evidence(
    path: impl AsRef<Path>,
    evidence: &BenchmarkEvidence,
) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create benchmark evidence directory {}", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(evidence)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("write benchmark evidence {}", path.display()))
}

fn benchmark_target() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64".into(),
        ("linux", "aarch64") => "linux-arm64".into(),
        ("macos", "x86_64") => "darwin-x64".into(),
        ("macos", "aarch64") => "darwin-arm64".into(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

fn benchmark_system() -> BenchmarkSystem {
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

fn valid_source_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn format_report(report: &BenchReport) -> String {
    let start = report.cold_start_p50_ms();
    let memory = report.idle_memory_mb();
    let size = report.artifact_mb();
    let start_ok = start <= COLD_START_MS;
    let memory_ok =
        memory <= IDLE_MEMORY_MB && (!cfg!(target_os = "linux") || report.memory_kind == "pss");
    let size_ok = size <= ARTIFACT_MB;
    format!(
        "§30 hello-service ({})\n\
         gate                 measured              limit     result\n\
         cold_start_p50_ms    {start:>8.2}              {COLD_START_MS:>4}      {}\n\
         idle_{}_mb         {memory:>8.2}              {IDLE_MEMORY_MB:>4}      {}\n\
         artifact_mb          {size:>8.2}              {ARTIFACT_MB:>4}      {}\n",
        std::env::consts::OS,
        if start_ok { "pass" } else { "fail" },
        report.memory_kind,
        if memory_ok { "pass" } else { "fail" },
        if size_ok { "pass" } else { "fail" },
    )
}

pub fn gates_passed(report: &BenchReport) -> bool {
    let memory_ok = report.idle_memory_mb() <= IDLE_MEMORY_MB
        && (!cfg!(target_os = "linux") || report.memory_kind == "pss");
    report.cold_start_p50_ms() <= COLD_START_MS && memory_ok && report.artifact_mb() <= ARTIFACT_MB
}

fn timed_cold_start(bin: &Path) -> Result<Duration> {
    let started = Instant::now();
    let mut child = spawn_service(bin)?;
    let listen = wait_listen(&mut child, Duration::from_secs(5));
    let elapsed = started.elapsed();
    stop_child(&mut child);
    listen?;
    Ok(elapsed)
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_service(bin: &Path) -> Result<Child> {
    Command::new(bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))
}

fn wait_listen(child: &mut Child, timeout: Duration) -> Result<String> {
    let stdout = child.stdout.take().context("missing stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut announced = false;
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if !announced && let Some(rest) = line.strip_prefix("tysel listen ") {
                let _ = tx.send(Ok(rest.trim().to_owned()));
                announced = true;
            }
        }
        if !announced {
            let _ = tx.send(Err(anyhow!("service exited before listen")));
        }
    });
    rx.recv_timeout(timeout).map_err(|_| anyhow!("timed out waiting for listen"))?
}

fn process_memory_kb(pid: u32) -> Result<(u64, &'static str)> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/smaps_rollup");
        let text = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
        let kb =
            pss_kb_from_smaps_rollup(&text).ok_or_else(|| anyhow!("Pss: missing from {path}"))?;
        Ok((kb, "pss"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output()?;
        if !output.status.success() {
            return Err(anyhow!("ps failed"));
        }
        let kb = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>()?;
        Ok((kb, "rss"))
    }
}

pub(crate) fn pss_kb_from_smaps_rollup(text: &str) -> Option<u64> {
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if key != "Pss" {
            continue;
        }
        return rest.split_whitespace().next()?.parse().ok();
    }
    None
}

fn percentile(samples: &[f64], q: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

pub fn find_stub() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("TYSEL_STUB") {
        return Ok(PathBuf::from(path));
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("tysel-service"));
    }
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let dir = PathBuf::from(dir);
        candidates.push(dir.join("release/tysel-service"));
        candidates.push(dir.join("debug/tysel-service"));
    }
    candidates.push(workspace_root().join("target/release/tysel-service"));
    candidates.push(workspace_root().join("target/debug/tysel-service"));
    for mut candidate in candidates {
        if cfg!(windows) {
            candidate.set_extension("exe");
        }
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "tysel-service stub not found; build with `cargo build -p tysel-runtime --bin tysel-service --release` or set TYSEL_STUB"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn percentile_picks_middle_sample() {
        assert_eq!(percentile(&[1.0, 10.0, 3.0], 0.5), 3.0);
    }

    #[test]
    fn pss_ignores_pss_dirty_and_reads_total() {
        let text = "\
Rss:                 8000 kB
Pss_Dirty:            999 kB
Pss:                 4321 kB
Pss_Anon:            1111 kB
";
        assert_eq!(pss_kb_from_smaps_rollup(text), Some(4321));
    }

    #[test]
    fn linux_gate_requires_pss_kind() {
        let report = BenchReport {
            artifact_bytes: 1_000_000,
            artifact_sha256: "00".repeat(32),
            cold_start_ms: vec![8.0; 11],
            idle_memory_kb: 10 * 1024,
            memory_kind: "rss",
        };
        if cfg!(target_os = "linux") {
            assert!(!gates_passed(&report));
        } else {
            assert!(gates_passed(&report));
        }
    }

    #[test]
    fn benchmark_evidence_contains_raw_samples_and_provenance() {
        let report = BenchReport {
            artifact_bytes: 1_000_000,
            artifact_sha256: "ab".repeat(32),
            cold_start_ms: vec![8.0; 11],
            idle_memory_kb: 10 * 1024,
            memory_kind: if cfg!(target_os = "linux") { "pss" } else { "rss" },
        };
        let evidence = benchmark_evidence(
            &report,
            "0123456789abcdef0123456789abcdef01234567",
            "cargo run -p tysel-testkit --bin tysel-bench --release",
        )
        .expect("evidence");
        assert_eq!(evidence.evidence_version, BENCHMARK_EVIDENCE_VERSION);
        assert_eq!(evidence.measurements.cold_start_ms.len(), 11);
        assert!(evidence.measurements.cold_start_p50_ms.passed);
        assert!(evidence.measurements.idle_memory_mb.passed);
        assert!(evidence.measurements.artifact_mb.passed);
    }

    #[test]
    fn benchmark_evidence_rejects_ambiguous_provenance() {
        let report = BenchReport {
            artifact_bytes: 1,
            artifact_sha256: "00".repeat(32),
            cold_start_ms: vec![1.0; 11],
            idle_memory_kb: 1,
            memory_kind: "pss",
        };
        assert!(benchmark_evidence(&report, "main", "bench").is_err());
        assert!(
            benchmark_evidence(&report, "0123456789abcdef0123456789abcdef01234567", "bench\nother")
                .is_err()
        );
    }
}
