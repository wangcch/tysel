use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use tysel_durable::{DurableStore, EventKind, POSTGRES_URL_ENV, PostgresStore, SqliteStore};
use tysel_engine::{IsolateConfig, Value};
use tysel_engine_qjs::{DurableSession, eval_durable};
use tysel_runtime::{DurableDispatcher, DurableRunStatus};
use tysel_task::TaskId;

use crate::report::{
    BenchScale, MetricReport, SuiteReport, gated_metric, metric, skipped_metric, suite_report,
    timed_ms,
};

pub fn run_durable(scale: BenchScale) -> Result<SuiteReport> {
    let mut metrics = Vec::new();
    metrics.push(sqlite_append(scale)?);
    metrics.push(postgres_append(scale)?);
    metrics.extend(suspend_resume(scale)?);
    metrics.extend(replay_effects(scale)?);
    metrics.push(signal_delivery(scale)?);
    metrics.push(restart_recovery(scale)?);
    metrics.push(effect_not_repeated(scale)?);
    Ok(suite_report("durable", metrics))
}

fn sqlite_append(scale: BenchScale) -> Result<MetricReport> {
    let store = SqliteStore::in_memory()?;
    let samples = timed_ms(scale.samples, || {
        append_n(&store, next_id(), 32)?;
        Ok(())
    })?;
    Ok(metric("sqlite_append_ms", "ms", samples))
}

fn postgres_append(scale: BenchScale) -> Result<MetricReport> {
    let url = std::env::var(POSTGRES_URL_ENV)
        .ok()
        .filter(|url| !url.is_empty())
        .or_else(|| std::env::var("TYSEL_POSTGRES_TEST_URL").ok().filter(|url| !url.is_empty()));
    let Some(url) = url else {
        return Ok(skipped_metric(
            "postgres_append_ms",
            "ms",
            format!("{POSTGRES_URL_ENV} is unset"),
        ));
    };
    let store = PostgresStore::connect(&url).context("connect durable postgres")?;
    let samples = timed_ms(scale.samples, || {
        append_n(&store, next_id(), 32)?;
        Ok(())
    })?;
    Ok(metric("postgres_append_ms", "ms", samples))
}

fn suspend_resume(scale: BenchScale) -> Result<Vec<MetricReport>> {
    let script = r#"(async () => { await tysel.durable.sleep("1ms"); return "awake"; })()"#;
    let mut suspend = Vec::with_capacity(scale.samples);
    let mut resume = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let store = Arc::new(SqliteStore::in_memory()?);
        let dispatcher = dispatcher(store.clone());
        let id = next_id();
        let started = Instant::now();
        let run = dispatcher.start(id, script);
        ensure!(matches!(run.result, Ok(DurableRunStatus::Suspended)), "sleep should suspend");
        suspend.push(started.elapsed().as_secs_f64() * 1_000.0);

        thread::sleep(Duration::from_millis(2));
        let started = Instant::now();
        let resumed = dispatcher.dispatch_task(id, script)?.expect("due wakeup");
        ensure!(
            matches!(resumed.result, Ok(DurableRunStatus::Completed(Value::String(ref value))) if value == "awake"),
            "sleep should resume: {:?}",
            resumed.result
        );
        resume.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(vec![metric("suspend_ms", "ms", suspend), gated_metric("resume_ms", "ms", resume, 10.0)])
}

fn replay_effects(scale: BenchScale) -> Result<Vec<MetricReport>> {
    let mut metrics = Vec::new();
    for &count in scale.durable_replay {
        let script = effect_script(count);
        let mut samples = Vec::with_capacity(scale.samples);
        for _ in 0..scale.samples {
            let store = Arc::new(SqliteStore::in_memory()?);
            let id = next_id();
            let first = eval_durable(
                &script,
                config(),
                DurableSession::new(store.clone(), id).map_err(anyhow::Error::msg)?,
            )?;
            ensure!(first == Value::Number(count as f64), "first effect run records callbacks");

            let started = Instant::now();
            let replayed = eval_durable(
                &script,
                config(),
                DurableSession::new(store, id).map_err(anyhow::Error::msg)?,
            )?;
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            ensure!(replayed == Value::Number(0.0), "replay must not rerun completed effects");
        }
        metrics.push(metric(format!("replay_{count}_effects_ms"), "ms", samples));
    }
    Ok(metrics)
}

fn signal_delivery(scale: BenchScale) -> Result<MetricReport> {
    let script = r#"(async () => JSON.stringify(await tysel.durable.waitForSignal("go")))()"#;
    let samples = timed_ms(scale.samples, || {
        let store = Arc::new(SqliteStore::in_memory()?);
        let dispatcher = dispatcher(store.clone());
        let id = next_id();
        let run = dispatcher.start(id, script);
        ensure!(matches!(run.result, Ok(DurableRunStatus::Suspended)));
        store.send_signal(id, "go", &serde_json::json!({"ok":true}), now_ms()?)?;
        let resumed = dispatcher.dispatch_task(id, script)?.expect("signal wakeup");
        match resumed.result {
            Ok(DurableRunStatus::Completed(Value::String(payload))) => {
                ensure!(payload.contains("\"ok\":true"), "signal payload: {payload}");
            }
            other => anyhow::bail!("signal resume: {other:?}"),
        }
        Ok(())
    })?;
    Ok(metric("signal_delivery_ms", "ms", samples))
}

fn restart_recovery(scale: BenchScale) -> Result<MetricReport> {
    let dir = TempBenchDir::new()?;
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let path = dir.path().join(format!("recover-{}.db", next_id().0));
        let id = next_id();
        {
            let store = SqliteStore::open(&path)?;
            append_n(&store, id, 16)?;
        }

        let started = Instant::now();
        let store = SqliteStore::open(&path)?;
        let history = store.load_history(id)?;
        ensure!(history.events.len() == 16);
        let mut cursor = history.replay();
        for index in 0..16 {
            ensure!(cursor.consume(EventKind::Effect, &format!("e{index}"))?.is_some());
        }
        cursor.ensure_consumed()?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(metric("restart_recovery_ms", "ms", samples))
}

struct TempBenchDir(std::path::PathBuf);

impl TempBenchDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-bench-{}-{}",
            std::process::id(),
            next_id().0
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempBenchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn effect_not_repeated(scale: BenchScale) -> Result<MetricReport> {
    let script = r#"
        (async () => {
            let calls = 0;
            const value = await tysel.durable.effect("once", async () => {
                calls += 1;
                return { answer: 42 };
            });
            return JSON.stringify({ value, calls });
        })()
    "#;
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let store = Arc::new(SqliteStore::in_memory()?);
        let id = next_id();
        let first = eval_durable(
            script,
            config(),
            DurableSession::new(store.clone(), id).map_err(anyhow::Error::msg)?,
        )?;
        ensure!(first == Value::String(r#"{"value":{"answer":42},"calls":1}"#.into()));

        let started = Instant::now();
        let replayed = eval_durable(
            script,
            config(),
            DurableSession::new(store.clone(), id).map_err(anyhow::Error::msg)?,
        )?;
        ensure!(
            replayed == Value::String(r#"{"value":{"answer":42},"calls":0}"#.into()),
            "completed effect ran again"
        );
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        ensure!(store.load_history(id)?.events.len() == 1);
    }
    Ok(metric("effect_replay_skips_callback_ms", "ms", samples))
}

fn append_n(store: &impl DurableStore, id: TaskId, count: u64) -> Result<()> {
    for sequence in 0..count {
        store.append_event_json_at(
            id,
            sequence,
            EventKind::Effect,
            format!("e{sequence}"),
            &sequence.to_string(),
            sequence,
        )?;
    }
    Ok(())
}

fn effect_script(count: usize) -> String {
    format!(
        r#"(async () => {{
            let calls = 0;
            for (let i = 0; i < {count}; i++) {{
                await tysel.durable.effect("e" + i, async () => {{ calls += 1; return i; }});
            }}
            return calls;
        }})()"#
    )
}

fn dispatcher(store: Arc<SqliteStore>) -> DurableDispatcher {
    DurableDispatcher::new(store, "bench-runner", 2_000, config()).expect("dispatcher")
}

fn config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 500,
        cpu_ms_per_turn: 200,
        memory_limit_bytes: 16 * 1024 * 1024,
    }
}

fn next_id() -> TaskId {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    TaskId((u128::from(std::process::id()) << 64) ^ nanos)
}

fn now_ms() -> Result<u64> {
    Ok(u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_suite_covers_required_metrics() {
        let report = run_durable(BenchScale::quick()).expect("durable bench");
        assert_eq!(report.suite, "durable");
        for name in [
            "sqlite_append_ms",
            "postgres_append_ms",
            "suspend_ms",
            "resume_ms",
            "replay_20_effects_ms",
            "replay_50_effects_ms",
            "signal_delivery_ms",
            "restart_recovery_ms",
            "effect_replay_skips_callback_ms",
        ] {
            assert!(report.metrics.iter().any(|metric| metric.name == name), "missing {name}");
        }
        let sqlite =
            report.metrics.iter().find(|metric| metric.name == "sqlite_append_ms").unwrap();
        assert!(!sqlite.samples.is_empty());
        let postgres =
            report.metrics.iter().find(|metric| metric.name == "postgres_append_ms").unwrap();
        if postgres.status.as_deref() == Some("skipped") {
            assert!(postgres.samples.is_empty());
            assert!(postgres.p50.is_none());
        } else {
            assert!(!postgres.samples.is_empty());
        }
    }

    #[test]
    fn completed_effects_do_not_rerun() {
        effect_not_repeated(BenchScale::quick()).expect("effect idempotence");
    }
}
