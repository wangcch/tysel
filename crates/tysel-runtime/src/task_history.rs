use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tysel_task::{Task, TaskId, TaskTrigger};
use tysel_task_rpc::TaskOutcome;

pub(crate) const MAX_HISTORY_TASKS: usize = 1024;
pub(crate) const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
const HISTORY_TTL: Duration = Duration::from_secs(60);

pub(crate) struct CompletedTask {
    pub task: Task,
    pub outcome: TaskOutcome,
    recorded: Instant,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct TaskHistory {
    entries: HashMap<TaskId, CompletedTask>,
    order: VecDeque<TaskId>,
    bytes: usize,
}

impl TaskHistory {
    pub fn get(&self, id: TaskId) -> Option<&CompletedTask> {
        self.entries.get(&id).filter(|entry| entry.recorded.elapsed() < HISTORY_TTL)
    }

    pub fn insert(&mut self, task: Task, outcome: TaskOutcome) {
        let now = Instant::now();
        self.prune(now);
        let bytes = task_bytes(&task) + outcome_bytes(&outcome);
        // A result may be delivered directly to a waiting caller even when it
        // is too large for historical lookup.
        if bytes > MAX_HISTORY_BYTES {
            return;
        }
        while self.entries.len() >= MAX_HISTORY_TASKS || self.bytes + bytes > MAX_HISTORY_BYTES {
            self.pop_front();
        }
        let id = task.meta.id;
        self.bytes += bytes;
        self.order.push_back(id);
        self.entries.insert(id, CompletedTask { task, outcome, recorded: now, bytes });
    }

    pub fn prune(&mut self, now: Instant) {
        while self.order.front().is_some_and(|id| {
            now.saturating_duration_since(self.entries[id].recorded) >= HISTORY_TTL
        }) {
            self.pop_front();
        }
    }

    fn pop_front(&mut self) {
        if let Some(id) = self.order.pop_front() {
            let entry = self.entries.remove(&id).expect("history order matches entries");
            self.bytes -= entry.bytes;
        }
    }
}

// Charge owned buffers, vector capacity and conservative per-object entry
// overhead. This is a retention budget, not an allocator/RSS measurement.
fn json_bytes(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    match value {
        Value::String(value) => value.capacity(),
        Value::Array(values) => {
            values.capacity() * size_of::<Value>() + values.iter().map(json_bytes).sum::<usize>()
        }
        Value::Object(values) => {
            values.iter().map(|(key, value)| 128 + key.capacity() + json_bytes(value)).sum()
        }
        _ => 0,
    }
}

fn task_bytes(task: &Task) -> usize {
    let option_bytes = |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
    let trigger = match &task.trigger {
        TaskTrigger::Http { method, path } => method.capacity() + path.capacity(),
        TaskTrigger::Cron { name, expression } => name.capacity() + expression.capacity(),
        TaskTrigger::Queue { name, handler, message_id } => {
            name.capacity() + handler.capacity() + option_bytes(message_id)
        }
        TaskTrigger::Mcp { tool } => tool.capacity(),
        TaskTrigger::Agent { name } => name.capacity(),
    };
    size_of::<CompletedTask>()
        + 128
        + task.meta.application_id.capacity()
        + option_bytes(&task.meta.tenant_id)
        + option_bytes(&task.meta.idempotency_key)
        + option_bytes(&task.meta.trace_id)
        + trigger
        + json_bytes(&task.input)
}

fn outcome_bytes(outcome: &TaskOutcome) -> usize {
    match outcome {
        TaskOutcome::Completed { result } => json_bytes(result),
        TaskOutcome::Failed { error, .. } => error.capacity(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_task::{TaskMeta, TaskState};

    fn task(id: u128, bytes: usize) -> Task {
        let mut task = Task::new(
            TaskMeta {
                id: TaskId(id),
                application_id: "test".into(),
                tenant_id: None,
                idempotency_key: None,
                trace_id: None,
            },
            TaskTrigger::Agent { name: "test".into() },
            None,
        )
        .with_input("x".repeat(bytes).into());
        task.state = TaskState::Completed;
        task
    }

    #[test]
    fn history_bounds_count_bytes_and_expires_when_idle() {
        let mut history = TaskHistory::default();
        for id in 0..10_000 {
            history.insert(task(id, 0), TaskOutcome::Completed { result: true.into() });
        }
        assert_eq!(history.entries.len(), MAX_HISTORY_TASKS);
        assert!(history.get(TaskId(0)).is_none());
        for id in 10_000..10_100 {
            history.insert(
                task(id, 128 * 1024),
                TaskOutcome::Completed { result: "x".repeat(128 * 1024).into() },
            );
            assert!(history.bytes <= MAX_HISTORY_BYTES);
        }
        assert!(history.entries.len() < 32);
        history.prune(Instant::now() + HISTORY_TTL);
        assert_eq!(history.bytes, 0);
        assert!(history.entries.is_empty() && history.order.is_empty());
        history.insert(task(11_000, MAX_HISTORY_BYTES + 1), TaskOutcome::Canceled {});
        assert!(history.entries.is_empty());
    }
}
