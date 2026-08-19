//! Unified internal task model.
//!
//! HTTP requests, cron, queues, MCP tools, and agent work all enter:
//! Trigger → Task → Policy → Scheduler → Capability → Result.

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u128);

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
}

#[derive(Debug, Clone)]
pub struct TaskMeta {
    pub id: TaskId,
    pub application_id: String,
    pub tenant_id: Option<String>,
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_created() {
        assert_eq!(TaskState::Created, TaskState::Created);
    }
}
