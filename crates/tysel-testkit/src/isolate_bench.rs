use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use tysel_engine::HttpRequest;
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::IsolatePool;
use tysel_isolate::{IsolatedHttpPool, Supervisor, WorkerSpec};

use crate::report::{BenchScale, SuiteReport, gated_metric, metric, suite_report, timed_ms};
use crate::{ensure_worker, process_memory_kb};

const HANDLER: &str = r#"
export default {
  async fetch() {
    return new Response("ok");
  },
};
"#;

pub fn run_isolate(scale: BenchScale) -> Result<SuiteReport> {
    let worker = ensure_worker().context("locate tysel-worker for isolate bench")?;
    run_isolate_with_worker(scale, &worker)
}

pub fn run_isolate_with_worker(scale: BenchScale, worker: &std::path::Path) -> Result<SuiteReport> {
    let mut metrics = Vec::new();

    let mut cold = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        let supervisor = Supervisor::spawn(worker, spec(), HashMap::new()).context("cold spawn")?;
        cold.push(started.elapsed().as_secs_f64() * 1_000.0);
        drop(supervisor);
    }
    metrics.push(metric("cold_create_ms", "ms", cold));

    let mut warm_create = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        let isolate =
            IsolatePool::spawn(1, HANDLER, in_process_config()).context("warm isolate create")?;
        warm_create.push(started.elapsed().as_secs_f64() * 1_000.0);
        drop(isolate);
    }
    metrics.push(gated_metric("warm_create_ms", "ms", warm_create, 5.0));

    let pool = IsolatedHttpPool::spawn(worker, HANDLER, spec(), Vec::new()).context("warm pool")?;
    let request = HttpRequest {
        method: "GET".into(),
        url: "http://tysel.local/".into(),
        headers: Vec::new(),
        body: Vec::new(),
        request_id: 0,
    };
    let _ = pool.dispatch_sync(request.clone()).context("warm-up acquire")?;
    let warm = timed_ms(scale.samples, || {
        let (head, body) = pool.dispatch_sync(request.clone())?;
        ensure!(head.status == 200 && body == b"ok", "warm pool response");
        Ok(())
    })?;
    metrics.push(metric("warm_pool_acquire_ms", "ms", warm));
    drop(pool);

    let idle_supervisor =
        Supervisor::spawn(worker, spec(), HashMap::new()).context("idle spawn")?;
    let pid = idle_supervisor.worker_pid().context("idle worker pid")?;
    thread::sleep(Duration::from_millis(200));
    let (idle_kb, kind) = process_memory_kb(pid).context("idle isolate memory")?;
    let mut idle = metric("idle_memory_kb", "KB", vec![idle_kb as f64]);
    idle.extra = Some(serde_json::json!({ "kind": kind }));
    metrics.push(idle);

    for &reuse in scale.isolate_reuse {
        let growth = reuse_growth(worker, reuse)?;
        metrics.push(metric(format!("reuse_{reuse}_growth_kb"), "KB", vec![growth]));
    }

    let timeout = timed_ms(scale.samples, || timeout_reclaim(worker))?;
    metrics.push(metric("timeout_reclaim_ms", "ms", timeout));

    let crash = timed_ms(scale.samples, || crash_replace(worker))?;
    metrics.push(metric("crash_replace_ms", "ms", crash));

    drop(idle_supervisor);
    Ok(suite_report("isolate", metrics))
}

fn spec() -> WorkerSpec {
    WorkerSpec { cpu_ms_per_turn: 2_000, request_timeout_ms: 2_000, ..WorkerSpec::default() }
}

fn in_process_config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 2_000,
        cpu_ms_per_turn: 2_000,
        memory_limit_bytes: 32 * 1024 * 1024,
    }
}

fn reuse_growth(worker: &std::path::Path, rounds: usize) -> Result<f64> {
    let mut supervisor =
        Supervisor::spawn(worker, spec(), HashMap::new()).context("reuse spawn")?;
    let pid = supervisor.worker_pid().context("reuse worker pid")?;
    thread::sleep(Duration::from_millis(100));
    let (before, _) = process_memory_kb(pid)?;
    for _ in 0..rounds {
        let value = supervisor.eval("1 + 1").context("reuse eval")?;
        ensure!(matches!(value, tysel_engine::Value::Number(n) if (n - 2.0).abs() < f64::EPSILON));
    }
    thread::sleep(Duration::from_millis(100));
    let pid = supervisor.worker_pid().context("reuse worker pid after eval")?;
    let (after, _) = process_memory_kb(pid)?;
    Ok(after.saturating_sub(before) as f64)
}

fn timeout_reclaim(worker: &std::path::Path) -> Result<()> {
    let mut supervisor = Supervisor::spawn(
        worker,
        WorkerSpec { request_timeout_ms: 80, cpu_ms_per_turn: 2_000, ..WorkerSpec::default() },
        HashMap::new(),
    )?;
    let err = supervisor
        .eval("(async () => tysel.sleep(5000))()")
        .expect_err("timeout should interrupt the in-flight turn");
    let message = err.to_string().to_ascii_lowercase();
    ensure!(message.contains("timeout") || message.contains("interrupt"), "timeout: {err}");
    let value = supervisor.eval("1 + 1").context("eval after timeout")?;
    ensure!(matches!(value, tysel_engine::Value::Number(n) if (n - 2.0).abs() < f64::EPSILON));
    Ok(())
}

fn crash_replace(worker: &std::path::Path) -> Result<()> {
    let mut supervisor = Supervisor::spawn(worker, spec(), HashMap::new())?;
    supervisor.eval("1 + 1").context("pre-crash eval")?;
    supervisor.kill_worker().context("kill worker")?;
    let started = Instant::now();
    let value = supervisor.eval("1 + 1").context("eval after crash")?;
    ensure!(matches!(value, tysel_engine::Value::Number(n) if (n - 2.0).abs() < f64::EPSILON));
    ensure!(started.elapsed() < Duration::from_secs(5), "crash replacement exceeded 5s");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetricReport;

    #[test]
    fn isolate_suite_covers_required_metrics() {
        let report = run_isolate(BenchScale::quick()).expect("isolate bench");
        assert_eq!(report.suite, "isolate");
        for name in [
            "cold_create_ms",
            "warm_create_ms",
            "warm_pool_acquire_ms",
            "idle_memory_kb",
            "reuse_20_growth_kb",
            "reuse_50_growth_kb",
            "timeout_reclaim_ms",
            "crash_replace_ms",
        ] {
            let metric = named(&report.metrics, name);
            assert!(!metric.samples.is_empty(), "{name}");
            assert!(metric.p50.is_some(), "{name}");
        }
    }

    fn named<'a>(metrics: &'a [MetricReport], name: &str) -> &'a MetricReport {
        metrics
            .iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }
}
