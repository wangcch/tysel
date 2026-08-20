//! Bounded task queue with explicit cancellation, deadlines, and worker claims.

use std::collections::{HashMap, VecDeque};

use tysel_task::{Task, TaskId, TaskState, TaskTransitionError};

const MAX_LEASE_OWNER_BYTES: usize = 128;
const MAX_LEASE_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskClaim {
    pub task: Task,
    pub generation: u64,
    pub lease_owner: String,
    pub lease_until_ms: u64,
}

#[derive(Debug, Clone)]
struct ActiveClaim {
    generation: u64,
    lease_owner: String,
    lease_until_ms: u64,
}

#[derive(Debug)]
pub struct Scheduler {
    capacity: usize,
    queue: VecDeque<TaskId>,
    tasks: HashMap<TaskId, Task>,
    claims: HashMap<TaskId, ActiveClaim>,
    generations: HashMap<TaskId, u64>,
}

impl Scheduler {
    pub fn new(capacity: usize) -> Result<Self, SchedulerError> {
        if capacity == 0 {
            return Err(SchedulerError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            queue: VecDeque::with_capacity(capacity),
            tasks: HashMap::new(),
            claims: HashMap::new(),
            generations: HashMap::new(),
        })
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
        let Some(next) = self.peek_runnable(now_ms)? else {
            return Ok(None);
        };
        let id = next.meta.id;
        let queued = self.queue.pop_front().ok_or(SchedulerError::Unknown(id))?;
        debug_assert_eq!(queued, id);
        let task = self.tasks.get_mut(&id).ok_or(SchedulerError::Unknown(id))?;
        task.begin_attempt()?;
        Ok(Some(task.clone()))
    }

    /// Inspect the next runnable task without starting an attempt. Overdue
    /// queued tasks are finalized while searching, matching [`Self::claim`].
    pub fn peek_runnable(&mut self, now_ms: u64) -> Result<Option<Task>, SchedulerError> {
        loop {
            let Some(id) = self.queue.front().copied() else {
                return Ok(None);
            };
            let task = self.tasks.get(&id).ok_or(SchedulerError::Unknown(id))?;
            if !task.deadline_reached(now_ms) {
                return Ok(Some(task.clone()));
            }
            self.queue.pop_front();
            self.transition(id, TaskState::TimedOut)?;
        }
    }

    /// Claim one runnable task with an owner- and generation-fenced lease.
    pub fn claim_with_lease(
        &mut self,
        now_ms: u64,
        lease_owner: &str,
        lease_ms: u64,
    ) -> Result<Option<TaskClaim>, SchedulerError> {
        validate_lease(lease_owner, lease_ms)?;
        let lease_until_ms = now_ms.checked_add(lease_ms).ok_or(SchedulerError::TimeRange)?;
        let Some(task) = self.claim(now_ms)? else {
            return Ok(None);
        };
        let Some(generation) =
            self.generations.get(&task.meta.id).copied().unwrap_or(0).checked_add(1)
        else {
            self.transition(task.meta.id, TaskState::Retrying)?;
            self.transition(task.meta.id, TaskState::Queued)?;
            return Err(SchedulerError::GenerationExhausted);
        };
        self.generations.insert(task.meta.id, generation);
        let claim = ActiveClaim { generation, lease_owner: lease_owner.into(), lease_until_ms };
        self.claims.insert(task.meta.id, claim.clone());
        Ok(Some(TaskClaim {
            task,
            generation: claim.generation,
            lease_owner: claim.lease_owner,
            lease_until_ms,
        }))
    }

    /// Renew an exact, still-live claim. A stale generation or owner cannot
    /// extend work reclaimed by another worker.
    pub fn renew_claim(
        &mut self,
        claim: &TaskClaim,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<TaskClaim, SchedulerError> {
        validate_lease(&claim.lease_owner, lease_ms)?;
        let lease_until_ms = now_ms.checked_add(lease_ms).ok_or(SchedulerError::TimeRange)?;
        let (generation, lease_owner) = {
            let active = self.require_live_claim(claim, now_ms)?;
            active.lease_until_ms = lease_until_ms;
            (active.generation, active.lease_owner.clone())
        };
        let task = self
            .tasks
            .get(&claim.task.meta.id)
            .ok_or(SchedulerError::Unknown(claim.task.meta.id))?
            .clone();
        Ok(TaskClaim { task, generation, lease_owner, lease_until_ms })
    }

    /// Commit a state reached by a worker only while its exact lease is live.
    pub fn finish_claim(
        &mut self,
        claim: &TaskClaim,
        now_ms: u64,
        next: TaskState,
    ) -> Result<Task, SchedulerError> {
        if !matches!(
            next,
            TaskState::Suspended
                | TaskState::Completed
                | TaskState::Failed
                | TaskState::Canceled
                | TaskState::TimedOut
        ) {
            return Err(SchedulerError::InvalidClaimOutcome(next));
        }
        self.require_live_claim(claim, now_ms)?;
        self.transition(claim.task.meta.id, next)
    }

    /// Gracefully release a live claim back to the runnable queue. If the task
    /// deadline has elapsed, release records a timeout instead of retrying it.
    /// Queue backpressure leaves the original lease intact so the worker can
    /// renew or finish it rather than losing ownership between transitions.
    pub fn release_claim(
        &mut self,
        claim: &TaskClaim,
        now_ms: u64,
    ) -> Result<Task, SchedulerError> {
        self.require_live_claim(claim, now_ms)?;
        let id = claim.task.meta.id;
        if self.tasks.get(&id).ok_or(SchedulerError::Unknown(id))?.deadline_reached(now_ms) {
            return self.transition(id, TaskState::TimedOut);
        }
        if self.queue.len() >= self.capacity {
            return Err(SchedulerError::Full { capacity: self.capacity });
        }
        self.transition(id, TaskState::Retrying)?;
        self.transition(id, TaskState::Queued)
    }

    /// Requeue expired work after a worker crash. At most `limit` claims and
    /// available queue slots are processed in one call.
    pub fn requeue_expired(
        &mut self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        let mut expired: Vec<_> = self
            .claims
            .iter()
            .filter_map(|(id, claim)| (claim.lease_until_ms <= now_ms).then_some(*id))
            .collect();
        expired.sort_unstable();
        self.requeue_claims(expired, now_ms, limit)
    }

    /// Immediately reclaim live claims owned by a disconnected worker. A new
    /// claim receives a new generation, fencing work still running after a
    /// network partition.
    pub fn requeue_owner_claims(
        &mut self,
        lease_owner: &str,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        validate_lease_owner(lease_owner)?;
        let mut owned: Vec<_> = self
            .claims
            .iter()
            .filter_map(|(id, claim)| (claim.lease_owner == lease_owner).then_some(*id))
            .collect();
        owned.sort_unstable();
        self.requeue_claims(owned, now_ms, limit)
    }

    fn requeue_claims(
        &mut self,
        claims: Vec<TaskId>,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        let mut processed = Vec::new();
        for id in claims.into_iter().take(limit) {
            let deadline_reached =
                self.tasks.get(&id).ok_or(SchedulerError::Unknown(id))?.deadline_reached(now_ms);
            if deadline_reached {
                self.transition(id, TaskState::TimedOut)?;
            } else if self.queue.len() < self.capacity {
                self.transition(id, TaskState::Retrying)?;
                self.transition(id, TaskState::Queued)?;
            } else {
                continue;
            }
            processed.push(id);
        }
        Ok(processed)
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
        if matches!(next, TaskState::Queued | TaskState::Suspended | TaskState::Retrying)
            || next.is_terminal()
        {
            self.claims.remove(&id);
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

    pub fn remaining_capacity(&self) -> usize {
        self.capacity.saturating_sub(self.queue.len())
    }

    fn require_live_claim(
        &mut self,
        claim: &TaskClaim,
        now_ms: u64,
    ) -> Result<&mut ActiveClaim, SchedulerError> {
        let active = self.claims.get_mut(&claim.task.meta.id).ok_or(SchedulerError::LeaseLost)?;
        if active.generation != claim.generation
            || active.lease_owner != claim.lease_owner
            || active.lease_until_ms <= now_ms
        {
            return Err(SchedulerError::LeaseLost);
        }
        Ok(active)
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
    #[error(
        "lease owner must be 1..={MAX_LEASE_OWNER_BYTES} ASCII letters, digits, '.', '_', ':', '@', or '-'"
    )]
    InvalidLeaseOwner,
    #[error("lease duration must be 1..={MAX_LEASE_MS} milliseconds")]
    InvalidLeaseDuration,
    #[error("scheduler clock is outside the supported range")]
    TimeRange,
    #[error("task claim generation is exhausted")]
    GenerationExhausted,
    #[error("task claim is missing, expired, or fenced by a newer generation")]
    LeaseLost,
    #[error("worker claims cannot finish in state {0:?}")]
    InvalidClaimOutcome(TaskState),
    #[error(transparent)]
    InvalidTransition(#[from] TaskTransitionError),
}

fn validate_lease(lease_owner: &str, lease_ms: u64) -> Result<(), SchedulerError> {
    validate_lease_owner(lease_owner)?;
    if lease_ms == 0 || lease_ms > MAX_LEASE_MS {
        return Err(SchedulerError::InvalidLeaseDuration);
    }
    Ok(())
}

fn validate_lease_owner(lease_owner: &str) -> Result<(), SchedulerError> {
    if lease_owner.is_empty()
        || lease_owner.len() > MAX_LEASE_OWNER_BYTES
        || !lease_owner.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'@' | b'-')
        })
    {
        return Err(SchedulerError::InvalidLeaseOwner);
    }
    Ok(())
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

    #[test]
    fn leased_claim_renews_and_commits_with_exact_token() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(10, None)).unwrap();
        let claim = scheduler.claim_with_lease(100, "worker-a", 50).unwrap().unwrap();
        assert_eq!(claim.generation, 1);
        assert_eq!(claim.lease_until_ms, 150);
        let renewed = scheduler.renew_claim(&claim, 120, 80).unwrap();
        assert_eq!(renewed.lease_until_ms, 200);
        let finished = scheduler.finish_claim(&renewed, 199, TaskState::Completed).unwrap();
        assert_eq!(finished.state, TaskState::Completed);
        assert!(matches!(
            scheduler.finish_claim(&renewed, 199, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
    }

    #[test]
    fn worker_crash_requeues_with_new_generation_and_rejects_late_commit() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(11, None)).unwrap();
        let stale = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        assert_eq!(scheduler.requeue_expired(10, 1).unwrap(), vec![TaskId(11)]);
        let current = scheduler.claim_with_lease(10, "worker-b", 10).unwrap().unwrap();
        assert_eq!(current.generation, stale.generation + 1);
        assert!(matches!(
            scheduler.finish_claim(&stale, 11, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
        assert_eq!(
            scheduler.finish_claim(&current, 11, TaskState::Completed).unwrap().state,
            TaskState::Completed
        );
    }

    #[test]
    fn cancellation_fences_an_in_flight_worker() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(12, None)).unwrap();
        let claim = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        assert_eq!(scheduler.cancel(TaskId(12)).unwrap().state, TaskState::Canceled);
        assert!(matches!(
            scheduler.finish_claim(&claim, 1, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
        assert_eq!(scheduler.get(TaskId(12)).unwrap().state, TaskState::Canceled);
    }

    #[test]
    fn expired_claim_waits_for_queue_capacity_without_losing_ownership_record() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(13, None)).unwrap();
        let stale = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        scheduler.enqueue(task(14, None)).unwrap();
        assert!(scheduler.requeue_expired(10, 1).unwrap().is_empty());
        assert!(matches!(
            scheduler.finish_claim(&stale, 10, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
        scheduler.cancel(TaskId(14)).unwrap();
        assert_eq!(scheduler.requeue_expired(10, 1).unwrap(), vec![TaskId(13)]);
    }

    #[test]
    fn release_requeues_and_fences_the_released_generation() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(15, None)).unwrap();
        let released = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        let task = scheduler.release_claim(&released, 1).unwrap();
        assert_eq!(task.state, TaskState::Queued);
        let current = scheduler.claim_with_lease(1, "worker-b", 10).unwrap().unwrap();
        assert_eq!(current.generation, released.generation + 1);
        assert!(matches!(
            scheduler.finish_claim(&released, 2, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
    }

    #[test]
    fn expired_claim_past_task_deadline_times_out_even_when_queue_is_full() {
        let mut scheduler = Scheduler::new(2).unwrap();
        scheduler.enqueue(task(20, Some(10))).unwrap();
        scheduler.enqueue(task(19, None)).unwrap();
        let deadline_claim = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        let blocked_claim = scheduler.claim_with_lease(0, "worker-b", 10).unwrap().unwrap();
        scheduler.enqueue(task(21, None)).unwrap();
        scheduler.enqueue(task(22, None)).unwrap();

        assert_eq!(scheduler.requeue_expired(10, 2).unwrap(), vec![TaskId(20)]);
        assert_eq!(scheduler.get(TaskId(20)).unwrap().state, TaskState::TimedOut);
        assert_eq!(scheduler.get(TaskId(19)).unwrap().state, TaskState::Running);
        assert!(matches!(
            scheduler.finish_claim(&deadline_claim, 10, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
        assert!(matches!(
            scheduler.finish_claim(&blocked_claim, 10, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
    }

    #[test]
    fn leased_claims_reject_invalid_owners_durations_and_outcomes() {
        let mut scheduler = Scheduler::new(1).unwrap();
        scheduler.enqueue(task(23, None)).unwrap();
        assert!(matches!(
            scheduler.claim_with_lease(0, "worker spoofed", 1),
            Err(SchedulerError::InvalidLeaseOwner)
        ));
        assert!(matches!(
            scheduler.claim_with_lease(0, "worker-a", 0),
            Err(SchedulerError::InvalidLeaseDuration)
        ));
        let claim = scheduler.claim_with_lease(0, "worker-a", 10).unwrap().unwrap();
        assert!(matches!(
            scheduler.finish_claim(&claim, 1, TaskState::Queued),
            Err(SchedulerError::InvalidClaimOutcome(TaskState::Queued))
        ));
        assert_eq!(
            scheduler.finish_claim(&claim, 1, TaskState::Completed).unwrap().state,
            TaskState::Completed
        );
    }

    #[test]
    fn disconnect_requeues_owned_claims_and_fences_the_old_generation() {
        let mut scheduler = Scheduler::new(2).unwrap();
        scheduler.enqueue(task(30, None)).unwrap();
        scheduler.enqueue(task(31, None)).unwrap();
        let stale = scheduler.claim_with_lease(0, "worker-a", 1_000).unwrap().unwrap();
        let other = scheduler.claim_with_lease(0, "worker-b", 1_000).unwrap().unwrap();

        assert_eq!(scheduler.requeue_owner_claims("worker-a", 1, 10).unwrap(), vec![TaskId(30)]);
        let current = scheduler.claim_with_lease(1, "worker-c", 1_000).unwrap().unwrap();
        assert_eq!(current.generation, stale.generation + 1);
        assert!(matches!(
            scheduler.finish_claim(&stale, 2, TaskState::Completed),
            Err(SchedulerError::LeaseLost)
        ));
        assert_eq!(
            scheduler.finish_claim(&other, 2, TaskState::Completed).unwrap().state,
            TaskState::Completed
        );
    }
}
