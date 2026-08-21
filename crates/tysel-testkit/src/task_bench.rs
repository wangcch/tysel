use std::sync::atomic::{AtomicU64, Ordering};
use std::{ffi::OsStr, process::Command};

use anyhow::{Context, Result, bail, ensure};
use tysel_scheduler::Scheduler;
use tysel_task::{Task, TaskId, TaskMeta, TaskState, TaskTrigger};

use crate::current_process_memory_kb;
use crate::report::{BenchScale, MetricReport, SuiteReport, metric, suite_report, timed_ms};

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

pub fn run_task(scale: BenchScale) -> Result<SuiteReport> {
    let mut metrics = Vec::new();
    for &count in scale.task_enqueue {
        let samples = timed_ms(scale.samples, || {
            let mut scheduler = Scheduler::new(count.max(1))?;
            enqueue_n(&mut scheduler, count)?;
            ensure!(scheduler.pending_len() == count);
            Ok(())
        })?;
        metrics.push(metric(format!("enqueue_{count}_ms"), "ms", samples));
    }

    let claim_commit = timed_ms(scale.samples, || claim_commit_round(1_000))?;
    metrics.push(metric("claim_commit_1000_ms", "ms", claim_commit));

    let queue_claim = timed_ms(scale.samples, || {
        let mut scheduler = Scheduler::new(8)?;
        let id = enqueue_one(&mut scheduler, None)?;
        let claimed = scheduler.claim(1)?.expect("queued task");
        ensure!(claimed.meta.id == id);
        Ok(())
    })?;
    metrics.push(metric("queue_claim_ms", "ms", queue_claim));

    let cancel = timed_ms(scale.samples, || {
        let mut scheduler = Scheduler::new(8)?;
        let id = enqueue_one(&mut scheduler, None)?;
        ensure!(scheduler.cancel(id)?.state == TaskState::Canceled);
        Ok(())
    })?;
    metrics.push(metric("cancel_transition_ms", "ms", cancel));

    let timeout = timed_ms(scale.samples, || {
        let mut scheduler = Scheduler::new(8)?;
        enqueue_one(&mut scheduler, Some(1))?;
        ensure!(scheduler.claim(1)?.is_none());
        Ok(())
    })?;
    metrics.push(metric("deadline_transition_ms", "ms", timeout));

    let renew = timed_ms(scale.samples, || {
        let mut scheduler = Scheduler::new(8)?;
        enqueue_one(&mut scheduler, None)?;
        let claim = scheduler.claim_with_lease(10, "bench-worker", 1_000)?.expect("claim");
        let renewed = scheduler.renew_claim(&claim, 20, 1_000)?;
        ensure!(renewed.lease_until_ms > claim.lease_until_ms);
        Ok(())
    })?;
    metrics.push(metric("lease_renew_ms", "ms", renew));

    let crash = timed_ms(scale.samples, || {
        let mut scheduler = Scheduler::new(8)?;
        enqueue_one(&mut scheduler, None)?;
        scheduler.claim_with_lease(10, "crashed-worker", 5_000)?.expect("claim");
        let requeued = scheduler.requeue_owner_claims("crashed-worker", 20, 8)?;
        ensure!(requeued.len() == 1);
        ensure!(scheduler.claim(30)?.is_some());
        Ok(())
    })?;
    metrics.push(metric("crash_requeue_ms", "ms", crash));

    metrics.push(backpressure_cap(scale.task_enqueue.iter().copied().max().unwrap_or(10_000))?);
    Ok(suite_report("task", metrics))
}

fn backpressure_cap(capacity: usize) -> Result<MetricReport> {
    let exe = std::env::current_exe().context("locate benchmark executable")?;
    if exe.file_stem() == Some(OsStr::new("tysel")) {
        let output = Command::new(&exe)
            .env("TYSEL_INTERNAL_TASK_MEMORY", capacity.to_string())
            .output()
            .context("run isolated task memory sample")?;
        ensure!(
            output.status.success(),
            "isolated task memory sample failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return serde_json::from_slice(&output.stdout)
            .context("decode isolated task memory sample");
    }
    task_backpressure_memory(capacity)
}

pub fn task_backpressure_memory(capacity: usize) -> Result<MetricReport> {
    ensure!(capacity > 0 && capacity <= 1_000_000, "invalid task memory capacity");
    let (before, kind) = current_process_memory_kb()?;
    let mut scheduler = Scheduler::new(capacity)?;
    enqueue_n(&mut scheduler, capacity)?;
    let overflowing = queued_task(None);
    match scheduler.enqueue(overflowing) {
        Err(tysel_scheduler::SchedulerError::Full { capacity: full }) => {
            ensure!(full == capacity);
        }
        other => bail!("expected queue full, got {other:?}"),
    }
    let (after, _) = current_process_memory_kb()?;
    let delta_kb = after.saturating_sub(before);
    let mut metric = metric("backpressure_memory_delta_kb", "KB", vec![delta_kb as f64]);
    metric.extra = Some(serde_json::json!({
        "kind": kind,
        "capacity": capacity,
        "before_kb": before,
        "after_kb": after,
        "pending": scheduler.pending_len(),
    }));
    Ok(metric)
}

fn claim_commit_round(count: usize) -> Result<()> {
    let mut scheduler = Scheduler::new(count.max(1))?;
    enqueue_n(&mut scheduler, count)?;
    for _ in 0..count {
        let claim = scheduler.claim_with_lease(1, "bench-worker", 5_000)?.expect("claim");
        scheduler.finish_claim(&claim, 2, TaskState::Completed)?;
    }
    ensure!(scheduler.pending_len() == 0);
    Ok(())
}

fn enqueue_n(scheduler: &mut Scheduler, count: usize) -> Result<()> {
    for _ in 0..count {
        scheduler.enqueue(queued_task(None))?;
    }
    Ok(())
}

fn enqueue_one(scheduler: &mut Scheduler, deadline_ms: Option<u64>) -> Result<TaskId> {
    let task = queued_task(deadline_ms);
    let id = task.meta.id;
    scheduler.enqueue(task)?;
    Ok(id)
}

fn queued_task(deadline_ms: Option<u64>) -> Task {
    let id = TaskId(u128::from(NEXT_TASK.fetch_add(1, Ordering::Relaxed)));
    Task::new(
        TaskMeta {
            id,
            application_id: "bench".into(),
            tenant_id: None,
            idempotency_key: None,
            trace_id: None,
        },
        TaskTrigger::Queue { name: "bench".into(), handler: "echo".into(), message_id: None },
        deadline_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_suite_covers_required_metrics() {
        let report = run_task(BenchScale::quick()).expect("task bench");
        assert_eq!(report.suite, "task");
        for name in [
            "enqueue_100_ms",
            "enqueue_200_ms",
            "enqueue_400_ms",
            "claim_commit_1000_ms",
            "queue_claim_ms",
            "cancel_transition_ms",
            "deadline_transition_ms",
            "lease_renew_ms",
            "crash_requeue_ms",
            "backpressure_memory_delta_kb",
        ] {
            let metric = report
                .metrics
                .iter()
                .find(|metric| metric.name == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert!(!metric.samples.is_empty(), "{name}");
        }
    }
}
