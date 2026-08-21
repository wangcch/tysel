use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{BenchmarkSystem, benchmark_system, percentile, workspace_root};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchScale {
    pub samples: usize,
    pub isolate_reuse: &'static [usize],
    pub task_enqueue: &'static [usize],
    pub durable_replay: &'static [usize],
    pub http1_concurrency: &'static [usize],
    pub http2_concurrency: &'static [usize],
}

impl BenchScale {
    pub fn full() -> Self {
        Self {
            samples: 101,
            isolate_reuse: &[100, 1000],
            task_enqueue: &[100, 1_000, 10_000],
            durable_replay: &[100, 1_000],
            http1_concurrency: &[1, 10, 100],
            http2_concurrency: &[1, 10, 100, 1_000],
        }
    }

    pub fn quick() -> Self {
        Self {
            samples: 3,
            isolate_reuse: &[20, 50],
            task_enqueue: &[100, 200, 400],
            durable_replay: &[20, 50],
            http1_concurrency: &[1, 10, 20],
            http2_concurrency: &[1, 10, 20],
        }
    }

    pub fn from_env() -> Self {
        match std::env::var("TYSEL_BENCH_QUICK").as_deref() {
            Ok("1") | Ok("true") => Self::quick(),
            _ => Self::full(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricReport {
    pub name: String,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    pub samples: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p50: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p95: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p99: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    pub suite: String,
    pub commit: String,
    pub system: BenchmarkSystem,
    pub metrics: Vec<MetricReport>,
}

pub fn metric(name: impl Into<String>, unit: impl Into<String>, samples: Vec<f64>) -> MetricReport {
    let p50 = percentile(&samples, 0.50);
    let p95 = (samples.len() >= 20).then(|| percentile(&samples, 0.95));
    let p99 = (samples.len() >= 100).then(|| percentile(&samples, 0.99));
    MetricReport {
        name: name.into(),
        unit: unit.into(),
        status: None,
        reason: None,
        limit: None,
        passed: None,
        samples,
        p50: Some(p50),
        p95,
        p99,
        extra: None,
    }
}

pub fn gated_metric(
    name: impl Into<String>,
    unit: impl Into<String>,
    samples: Vec<f64>,
    limit: f64,
) -> MetricReport {
    let mut report = metric(name, unit, samples);
    let passed = report.p50.is_some_and(|measured| measured <= limit);
    report.status = Some(if passed { "pass" } else { "fail" }.into());
    report.limit = Some(limit);
    report.passed = Some(passed);
    report
}

pub fn skipped_metric(
    name: impl Into<String>,
    unit: impl Into<String>,
    reason: impl Into<String>,
) -> MetricReport {
    MetricReport {
        name: name.into(),
        unit: unit.into(),
        status: Some("skipped".into()),
        reason: Some(reason.into()),
        limit: None,
        passed: None,
        samples: Vec::new(),
        p50: None,
        p95: None,
        p99: None,
        extra: None,
    }
}

pub fn suite_report(suite: impl Into<String>, metrics: Vec<MetricReport>) -> SuiteReport {
    SuiteReport { suite: suite.into(), commit: git_commit(), system: benchmark_system(), metrics }
}

pub fn git_commit() -> String {
    let output =
        Command::new("git").args(["rev-parse", "HEAD"]).current_dir(workspace_root()).output();
    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        _ => String::new(),
    }
}

pub fn timed_ms(
    samples: usize,
    mut run: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Vec<f64>> {
    let mut out = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = std::time::Instant::now();
        run()?;
        out.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_keeps_metric_distributions_unambiguous() {
        let report =
            suite_report("isolate", vec![metric("cold_create_ms", "ms", vec![1.0, 2.0, 3.0])]);
        assert_eq!(report.suite, "isolate");
        assert_eq!(report.metrics[0].samples, vec![1.0, 2.0, 3.0]);
        assert_eq!(report.metrics[0].p50, Some(2.0));
        assert_eq!(report.metrics[0].p95, None);
        assert_eq!(report.metrics[0].p99, None);
        assert!(report.metrics[0].reason.is_none());
    }

    #[test]
    fn skipped_metrics_omit_placeholder_numbers() {
        let skipped =
            skipped_metric("postgres_append_ms", "ms", "TYSEL_DURABLE_POSTGRES_URL unset");
        let text = serde_json::to_string(&skipped).unwrap();
        assert!(!text.contains("\"p50\""));
        assert_eq!(skipped.status.as_deref(), Some("skipped"));
        assert!(skipped.samples.is_empty());
    }

    #[test]
    fn gated_metrics_record_threshold_and_decision() {
        let report = gated_metric("warm_create_ms", "ms", vec![1.0, 3.0, 7.0], 5.0);
        assert_eq!(report.p50, Some(3.0));
        assert_eq!(report.limit, Some(5.0));
        assert_eq!(report.passed, Some(true));
        assert_eq!(report.status.as_deref(), Some("pass"));
    }

    #[test]
    fn tail_percentiles_require_enough_samples() {
        let twenty = metric("latency", "ms", (0..20).map(f64::from).collect());
        assert!(twenty.p95.is_some());
        assert!(twenty.p99.is_none());
        let hundred = metric("latency", "ms", (0..100).map(f64::from).collect());
        assert!(hundred.p95.is_some());
        assert!(hundred.p99.is_some());
    }
}
