//! Shared helpers and the M0 §30 measurement harness (`tysel-bench`).

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tysel_build::{embed, read_bundle, tap_from_app};
use tysel_manifest::Manifest;

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

pub const COLD_START_MS: f64 = 15.0;
pub const IDLE_MEMORY_MB: f64 = 32.0;
pub const ARTIFACT_MB: f64 = 20.0;

#[derive(Debug, Clone)]
pub struct BenchReport {
    pub artifact_bytes: u64,
    pub cold_start_ms: Vec<f64>,
    pub idle_memory_kb: u64,
    pub memory_kind: &'static str,
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

pub fn package_hello_service(stub: &Path, output: &Path) -> Result<()> {
    let root = workspace_root();
    let app = root.join("examples/hello-service");
    let manifest = Manifest::from_path(app.join("tysel.toml")).context("hello-service manifest")?;
    let entry = app.join(&manifest.app.entry);
    let (bundle, source_map) = read_bundle(&entry).context("transpile hello-service")?;
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
    let artifact_bytes = fs::metadata(&packaged)?.len();

    // Discard two runs so dyld / page cache are not the first sample.
    for _ in 0..2 {
        let _ = timed_cold_start(&packaged)?;
    }
    let mut cold_start_ms = Vec::with_capacity(11);
    for _ in 0..11 {
        cold_start_ms.push(timed_cold_start(&packaged)?.as_secs_f64() * 1_000.0);
    }

    let mut child = spawn_service(&packaged)?;
    let _addr = wait_listen(&mut child, Duration::from_secs(5))?;
    thread::sleep(Duration::from_millis(400));
    let pid = child.id();
    let (idle_memory_kb, memory_kind) = process_memory_kb(pid).context("sample idle memory")?;
    let _ = child.kill();
    let _ = child.wait();

    Ok(BenchReport { artifact_bytes, cold_start_ms, idle_memory_kb, memory_kind })
}

pub fn format_report(report: &BenchReport) -> String {
    let start = report.cold_start_p50_ms();
    let memory = report.idle_memory_mb();
    let size = report.artifact_mb();
    let start_ok = start <= COLD_START_MS;
    let memory_ok = memory <= IDLE_MEMORY_MB;
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
    report.cold_start_p50_ms() <= COLD_START_MS
        && report.idle_memory_mb() <= IDLE_MEMORY_MB
        && report.artifact_mb() <= ARTIFACT_MB
}

fn timed_cold_start(bin: &Path) -> Result<Duration> {
    let started = Instant::now();
    let mut child = spawn_service(bin)?;
    wait_listen(&mut child, Duration::from_secs(5))?;
    let elapsed = started.elapsed();
    let _ = child.kill();
    let _ = child.wait();
    Ok(elapsed)
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
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(rest) = line.strip_prefix("tysel listen ") {
                let _ = tx.send(Ok(rest.trim().to_owned()));
                return;
            }
        }
        let _ = tx.send(Err(anyhow!("service exited before listen")));
    });
    rx.recv_timeout(timeout).map_err(|_| anyhow!("timed out waiting for listen"))?
}

fn process_memory_kb(pid: u32) -> Result<(u64, &'static str)> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("Pss:") {
                    let kb = rest.split_whitespace().next().context("pss value")?.parse::<u64>()?;
                    return Ok((kb, "pss"));
                }
            }
        }
    }
    let output = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output()?;
    if !output.status.success() {
        return Err(anyhow!("ps failed"));
    }
    let kb = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>()?;
    Ok((kb, "rss"))
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tysel-service"));
        }
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
}
