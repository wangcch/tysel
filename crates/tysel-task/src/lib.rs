//! Unified internal task model.
//!
//! HTTP requests, cron, queues, MCP tools, and agent work all enter:
//! Trigger → Task → Policy → Scheduler → Capability → Result.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u128);

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskTrigger {
    Http { method: String, path: String },
    Cron { name: String, expression: String },
    Queue { name: String, handler: String, message_id: Option<String> },
    Mcp { tool: String },
    Agent { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Created,
    Queued,
    Running,
    WaitingIo,
    Suspended,
    Retrying,
    Completed,
    Failed,
    Canceled,
    TimedOut,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled | Self::TimedOut)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use TaskState::*;
        matches!(
            (self, next),
            (Created, Queued | Canceled)
                | (Queued, Running | Canceled | TimedOut)
                | (
                    Running,
                    WaitingIo | Suspended | Retrying | Completed | Failed | Canceled | TimedOut
                )
                | (WaitingIo, Running | Suspended | Retrying | Failed | Canceled | TimedOut)
                | (Suspended | Retrying, Queued | Canceled | TimedOut)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskMeta {
    pub id: TaskId,
    pub application_id: String,
    pub tenant_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub meta: TaskMeta,
    pub trigger: TaskTrigger,
    pub input: serde_json::Value,
    pub state: TaskState,
    pub attempt: u32,
    /// Absolute scheduler clock value. The scheduler owns the clock's epoch.
    pub deadline_ms: Option<u64>,
}

impl Task {
    pub fn new(meta: TaskMeta, trigger: TaskTrigger, deadline_ms: Option<u64>) -> Self {
        Self {
            meta,
            trigger,
            input: serde_json::Value::Null,
            state: TaskState::Created,
            attempt: 0,
            deadline_ms,
        }
    }

    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }

    pub fn transition(&mut self, next: TaskState) -> Result<(), TaskTransitionError> {
        if !self.state.can_transition_to(next) {
            return Err(TaskTransitionError { id: self.meta.id, from: self.state, to: next });
        }
        self.state = next;
        Ok(())
    }

    pub fn begin_attempt(&mut self) -> Result<(), TaskTransitionError> {
        self.transition(TaskState::Running)?;
        self.attempt = self.attempt.saturating_add(1);
        Ok(())
    }

    pub fn deadline_reached(&self, now_ms: u64) -> bool {
        self.deadline_ms.is_some_and(|deadline| now_ms >= deadline)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("task {id} cannot transition from {from:?} to {to:?}")]
pub struct TaskTransitionError {
    pub id: TaskId,
    pub from: TaskState,
    pub to: TaskState,
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task::new(
            TaskMeta {
                id: TaskId(42),
                application_id: "orders".into(),
                tenant_id: Some("tenant-a".into()),
                idempotency_key: Some("order-7".into()),
                trace_id: Some("trace-9".into()),
            },
            TaskTrigger::Queue {
                name: "orders".into(),
                handler: "consume-order".into(),
                message_id: Some("message-1".into()),
            },
            Some(100),
        )
    }

    #[test]
    fn valid_lifecycle_reaches_completion() {
        let mut task = task();
        task.transition(TaskState::Queued).unwrap();
        task.begin_attempt().unwrap();
        task.transition(TaskState::WaitingIo).unwrap();
        task.transition(TaskState::Running).unwrap();
        task.transition(TaskState::Completed).unwrap();
        assert_eq!(task.attempt, 1);
        assert!(task.state.is_terminal());
    }

    #[test]
    fn retry_starts_another_attempt() {
        let mut task = task();
        task.transition(TaskState::Queued).unwrap();
        task.begin_attempt().unwrap();
        task.transition(TaskState::Retrying).unwrap();
        task.transition(TaskState::Queued).unwrap();
        task.begin_attempt().unwrap();
        assert_eq!(task.attempt, 2);
    }

    #[test]
    fn terminal_state_rejects_more_work() {
        let mut task = task();
        task.transition(TaskState::Canceled).unwrap();
        let err = task.transition(TaskState::Queued).unwrap_err();
        assert_eq!(err.from, TaskState::Canceled);
        assert_eq!(err.to, TaskState::Queued);
    }

    #[test]
    fn deadline_uses_inclusive_boundary() {
        let task = task();
        assert!(!task.deadline_reached(99));
        assert!(task.deadline_reached(100));
    }

    #[test]
    fn id_display_is_fixed_width_hex() {
        assert_eq!(TaskId(42).to_string(), "0000000000000000000000000000002a");
    }
}
