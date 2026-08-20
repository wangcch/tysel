//! Bounded task queue with explicit cancellation, deadlines, and worker claims.

use std::collections::{HashMap, VecDeque};

use tysel_task::{Task, TaskId, TaskState, TaskTransitionError};

#[derive(Debug)]
pub struct Scheduler {
    capacity: usize,
    queue: VecDeque<TaskId>,
    tasks: HashMap<TaskId, Task>,
}

impl Scheduler {
    pub fn new(capacity: usize) -> Result<Self, SchedulerError> {
        if capacity == 0 {
            return Err(SchedulerError::InvalidCapacity);
        }
        Ok(Self { capacity, queue: VecDeque::with_capacity(capacity), tasks: HashMap::new() })
    }

    pub fn enqueue(&mut self, mut task: Task) -> Result<(), SchedulerError> {
        let id = task.meta.id;
        if self.tasks.contains_key(&id) {
            return Err(SchedulerError::Duplicate(id));
        }
        if self.queue.len() >= self.capacity {
            return Err(SchedulerError::Full { capacity: self.capacity });
        }
        task.transition(TaskState::Queued)?;
        self.queue.push_back(id);
        self.tasks.insert(id, task);
        Ok(())
    }

    /// Claim the first runnable task, expiring overdue queued tasks along the way.
    pub fn claim(&mut self, now_ms: u64) -> Result<Option<Task>, SchedulerError> {
        while let Some(id) = self.queue.pop_front() {
            let task = self.tasks.get_mut(&id).ok_or(SchedulerError::Unknown(id))?;
            if task.deadline_reached(now_ms) {
                task.transition(TaskState::TimedOut)?;
                continue;
            }
            task.begin_attempt()?;
            return Ok(Some(task.clone()));
        }
        Ok(None)
    }

    /// Move a task to its next lifecycle state. Moving to `Queued` makes it
    /// runnable again and therefore observes queue backpressure.
    pub fn transition(&mut self, id: TaskId, next: TaskState) -> Result<Task, SchedulerError> {
        if !self.tasks.contains_key(&id) {
            return Err(SchedulerError::Unknown(id));
        }
        if next == TaskState::Queued && self.queue.len() >= self.capacity {
            return Err(SchedulerError::Full { capacity: self.capacity });
        }
        let task = self.tasks.get_mut(&id).expect("task existence checked above");
        task.transition(next)?;
        if next == TaskState::Queued {
            self.queue.push_back(id);
        } else if next.is_terminal() {
            self.queue.retain(|queued| *queued != id);
        }
        Ok(task.clone())
    }

    /// Cancellation is idempotent. A task that already finished keeps its
    /// original terminal result.
    pub fn cancel(&mut self, id: TaskId) -> Result<Task, SchedulerError> {
        let task = self.tasks.get(&id).ok_or(SchedulerError::Unknown(id))?;
        if task.state.is_terminal() {
            return Ok(task.clone());
        }
        self.transition(id, TaskState::Canceled)
    }

    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("scheduler capacity must be greater than zero")]
    InvalidCapacity,
    #[error("scheduler queue is full (capacity {capacity})")]
    Full { capacity: usize },
    #[error("task {0} is already registered")]
    Duplicate(TaskId),
    #[error("task {0} is not registered")]
    Unknown(TaskId),
    #[error(transparent)]
    InvalidTransition(#[from] TaskTransitionError),
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_task::{TaskMeta, TaskTrigger};

    fn task(id: u128, deadline_ms: Option<u64>) -> Task {
        Task::new(
            TaskMeta {
                id: TaskId(id),
                application_id: "test".into(),
                tenant_id: None,
                idempotency_key: None,
                trace_id: None,
            },
            TaskTrigger::Agent { name: format!("agent-{id}") },
            deadline_ms,
        )
    }

    #[test]
    fn rejects_zero_capacity() {
        assert!(matches!(Scheduler::new(0), Err(SchedulerError::InvalidCapacity)));
    }

    #[test]
    fn claims_tasks_in_fifo_order() {
        let mut scheduler = Scheduler::new(2).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        scheduler.enqueue(task(2, None)).unwrap();
        assert_eq!(scheduler.claim(0).unwrap().unwrap().meta.id, TaskId(1));
        assert_eq!(scheduler.claim(0).unwrap().unwrap().meta.id, TaskId(2));
        assert_eq!(scheduler.claim(0).unwrap(), None);
    }

    #[test]
    fn queue_capacity_applies_backpressure() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        let err = scheduler.enqueue(task(2, None)).unwrap_err();
        assert!(matches!(err, SchedulerError::Full { capacity: 1 }));
    }

    #[test]
    fn expired_task_is_skipped_and_recorded() {
        let mut scheduler = Scheduler::new(2).unwrap();
        scheduler.enqueue(task(1, Some(10))).unwrap();
        scheduler.enqueue(task(2, Some(20))).unwrap();
        let claimed = scheduler.claim(10).unwrap().unwrap();
        assert_eq!(claimed.meta.id, TaskId(2));
        assert_eq!(scheduler.get(TaskId(1)).unwrap().state, TaskState::TimedOut);
    }

    #[test]
    fn cancel_releases_queue_capacity_and_is_idempotent() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        assert_eq!(scheduler.cancel(TaskId(1)).unwrap().state, TaskState::Canceled);
        assert_eq!(scheduler.cancel(TaskId(1)).unwrap().state, TaskState::Canceled);
        scheduler.enqueue(task(2, None)).unwrap();
    }

    #[test]
    fn retry_requeues_and_increments_attempt() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        assert_eq!(scheduler.claim(0).unwrap().unwrap().attempt, 1);
        scheduler.transition(TaskId(1), TaskState::Retrying).unwrap();
        scheduler.transition(TaskId(1), TaskState::Queued).unwrap();
        assert_eq!(scheduler.claim(0).unwrap().unwrap().attempt, 2);
    }

    #[test]
    fn duplicate_ids_are_rejected_after_completion() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        scheduler.claim(0).unwrap();
        scheduler.transition(TaskId(1), TaskState::Completed).unwrap();
        assert!(matches!(
            scheduler.enqueue(task(1, None)),
            Err(SchedulerError::Duplicate(TaskId(1)))
        ));
    }

    #[test]
    fn unknown_task_is_reported_even_when_queue_is_full() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(1, None)).unwrap();
        assert!(matches!(
            scheduler.transition(TaskId(2), TaskState::Queued),
            Err(SchedulerError::Unknown(TaskId(2)))
        ));
    }
}
