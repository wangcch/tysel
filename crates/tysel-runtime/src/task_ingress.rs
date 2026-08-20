//! Bounded Cron and Queue ingress for registered module tasks.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::sync::Mutex;
use tysel_engine_qjs::{ModuleTaskDefinition, ModuleTaskKind};
use tysel_task::{Task, TaskId, TaskMeta, TaskTrigger};

use crate::{TaskRpcBroker, TaskRpcBrokerError};

const MINUTE_MS: u64 = 60_000;
const MAX_CRON_CATCH_UP_MINUTES: u64 = 24 * 60;

/// Validated five-field UTC cron expression. Lists, ranges and steps are
/// accepted; month and weekday names are deliberately excluded from v0.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpression {
    source: String,
    minute: Field,
    hour: Field,
    day: Field,
    month: Field,
    weekday: Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    minimum: u8,
    allowed: Vec<bool>,
    wildcard: bool,
}

impl Field {
    fn parse(source: &str, minimum: u8, maximum: u8) -> Result<Self, TaskIngressError> {
        if source.is_empty() {
            return Err(TaskIngressError::InvalidCron("empty cron field".into()));
        }
        let mut allowed = vec![false; usize::from(maximum) + 1];
        let wildcard = source == "*";
        for component in source.split(',') {
            let (range, step) = match component.split_once('/') {
                Some((range, step)) => {
                    let step = parse_number(step, minimum, maximum)?;
                    if step == 0 {
                        return Err(TaskIngressError::InvalidCron(
                            "cron step must be positive".into(),
                        ));
                    }
                    (range, step)
                }
                None => (component, 1),
            };
            let (start, end) = if range == "*" {
                (minimum, maximum)
            } else if let Some((start, end)) = range.split_once('-') {
                let start = parse_number(start, minimum, maximum)?;
                let end = parse_number(end, minimum, maximum)?;
                if start > end {
                    return Err(TaskIngressError::InvalidCron("cron range is reversed".into()));
                }
                (start, end)
            } else {
                let value = parse_number(range, minimum, maximum)?;
                (value, value)
            };
            for value in (start..=end).step_by(usize::from(step)) {
                allowed[usize::from(value)] = true;
            }
        }
        if !allowed.iter().any(|allowed| *allowed) {
            return Err(TaskIngressError::InvalidCron("cron field selects no values".into()));
        }
        Ok(Self { minimum, allowed, wildcard })
    }

    fn contains(&self, value: u8) -> bool {
        value >= self.minimum && self.allowed.get(usize::from(value)).copied().unwrap_or(false)
    }
}

fn parse_number(source: &str, minimum: u8, maximum: u8) -> Result<u8, TaskIngressError> {
    let value: u8 = source
        .parse()
        .map_err(|_| TaskIngressError::InvalidCron(format!("invalid cron value '{source}'")))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(TaskIngressError::InvalidCron(format!(
            "cron value {value} is outside {minimum}..={maximum}"
        )));
    }
    Ok(value)
}

impl CronExpression {
    pub fn parse(source: &str) -> Result<Self, TaskIngressError> {
        if source.len() > 128 {
            return Err(TaskIngressError::InvalidCron("cron expression is too long".into()));
        }
        let fields: Vec<_> = source.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(TaskIngressError::InvalidCron(
                "cron expression must contain five fields".into(),
            ));
        }
        Ok(Self {
            source: source.into(),
            minute: Field::parse(fields[0], 0, 59)?,
            hour: Field::parse(fields[1], 0, 23)?,
            day: Field::parse(fields[2], 1, 31)?,
            month: Field::parse(fields[3], 1, 12)?,
            weekday: Field::parse(fields[4], 0, 7)?,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Match an absolute Unix timestamp in UTC. As in traditional cron, day of
    /// month and day of week are ORed when both are restricted.
    pub fn matches_unix_ms(&self, unix_ms: u64) -> bool {
        let minute = unix_ms / MINUTE_MS;
        let days = minute / (24 * 60);
        let minute_of_day = minute % (24 * 60);
        let hour = (minute_of_day / 60) as u8;
        let minute = (minute_of_day % 60) as u8;
        let (_, month, day) = civil_from_days(days as i64);
        let weekday = ((days + 4) % 7) as u8;
        let day_match = self.day.contains(day);
        let weekday_match =
            self.weekday.contains(weekday) || (weekday == 0 && self.weekday.contains(7));
        let calendar_day_match = if self.day.wildcard || self.weekday.wildcard {
            day_match && weekday_match
        } else {
            day_match || weekday_match
        };
        self.minute.contains(minute)
            && self.hour.contains(hour)
            && self.month.contains(month)
            && calendar_day_match
    }
}

// Howard Hinnant's civil calendar conversion, with day zero at 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}

#[derive(Debug, Clone)]
struct CronRegistration {
    name: String,
    schedule: CronExpression,
}

/// Immutable routing table produced from a task module inspection.
#[derive(Debug, Clone, Default)]
pub struct TaskRegistry {
    cron: Vec<CronRegistration>,
    queues: BTreeMap<String, String>,
}

impl TaskRegistry {
    pub fn from_definitions(
        definitions: &[ModuleTaskDefinition],
    ) -> Result<Self, TaskIngressError> {
        let mut cron = Vec::new();
        let mut queues = BTreeMap::new();
        for definition in definitions {
            match &definition.kind {
                ModuleTaskKind::Cron { expression } => cron.push(CronRegistration {
                    name: definition.name.clone(),
                    schedule: CronExpression::parse(expression)?,
                }),
                ModuleTaskKind::Queue { queue } => {
                    if let Some(existing) = queues.insert(queue.clone(), definition.name.clone()) {
                        return Err(TaskIngressError::DuplicateQueueConsumer {
                            queue: queue.clone(),
                            first: existing,
                            second: definition.name.clone(),
                        });
                    }
                }
                ModuleTaskKind::Mcp { .. } => {}
            }
        }
        cron.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { cron, queues })
    }

    pub fn queue_handler(&self, queue: &str) -> Option<&str> {
        self.queues.get(queue).map(String::as_str)
    }

    pub fn cron_len(&self) -> usize {
        self.cron.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggeredTask {
    pub id: TaskId,
    pub handler: String,
    pub scheduled_at_ms: u64,
}

/// Concurrent ingress shared by queue producers and a single cron poller.
pub struct TaskIngress {
    broker: Arc<Mutex<TaskRpcBroker>>,
    registry: TaskRegistry,
    application_id: String,
    next_id: AtomicU64,
    last_cron_minute: Mutex<Option<u64>>,
    request_timeout_ms: u64,
}

impl TaskIngress {
    pub fn new(
        broker: Arc<Mutex<TaskRpcBroker>>,
        registry: TaskRegistry,
        application_id: impl Into<String>,
        first_task_id: u64,
        request_timeout_ms: u64,
    ) -> Result<Self, TaskIngressError> {
        let application_id = application_id.into();
        if application_id.is_empty() {
            return Err(TaskIngressError::InvalidApplicationId);
        }
        if first_task_id == 0 {
            return Err(TaskIngressError::InvalidTaskIdSeed);
        }
        if request_timeout_ms == 0 {
            return Err(TaskIngressError::InvalidTimeout);
        }
        Ok(Self {
            broker,
            registry,
            application_id,
            next_id: AtomicU64::new(first_task_id),
            last_cron_minute: Mutex::new(None),
            request_timeout_ms,
        })
    }

    pub fn registry(&self) -> &TaskRegistry {
        &self.registry
    }

    pub async fn enqueue_queue(
        &self,
        queue: &str,
        message_id: Option<String>,
        input: Value,
        now_ms: u64,
    ) -> Result<TaskId, TaskIngressError> {
        let handler = self
            .registry
            .queue_handler(queue)
            .ok_or_else(|| TaskIngressError::UnknownQueue(queue.into()))?;
        let id = TaskId(u128::from(self.reserve_task_ids(1)?));
        let deadline =
            now_ms.checked_add(self.request_timeout_ms).ok_or(TaskIngressError::Clock)?;
        let task = Task::new(
            TaskMeta {
                id,
                application_id: self.application_id.clone(),
                tenant_id: None,
                idempotency_key: message_id.clone(),
                trace_id: None,
            },
            TaskTrigger::Queue { name: queue.into(), handler: handler.into(), message_id },
            Some(deadline),
        )
        .with_input(input);
        self.broker.lock().await.enqueue(task)?;
        Ok(id)
    }

    /// Enqueue all due registrations since the previous call. Catch-up is
    /// bounded to one day so a stale process cannot monopolize the scheduler.
    pub async fn enqueue_due_cron(
        &self,
        now_ms: u64,
    ) -> Result<Vec<TriggeredTask>, TaskIngressError> {
        let current_minute = now_ms / MINUTE_MS;
        let mut last = self.last_cron_minute.lock().await;
        let start = match *last {
            Some(previous) if previous >= current_minute => return Ok(Vec::new()),
            Some(previous) => previous
                .saturating_add(1)
                .max(current_minute.saturating_sub(MAX_CRON_CATCH_UP_MINUTES - 1)),
            None => current_minute,
        };
        let mut pending = Vec::new();
        for minute in start..=current_minute {
            let scheduled_at_ms = minute.checked_mul(MINUTE_MS).ok_or(TaskIngressError::Clock)?;
            for registration in &self.registry.cron {
                if !registration.schedule.matches_unix_ms(scheduled_at_ms) {
                    continue;
                }
                pending.push((
                    minute,
                    registration.name.clone(),
                    registration.schedule.source().to_owned(),
                    scheduled_at_ms,
                ));
            }
        }
        let mut broker = self.broker.lock().await;
        let available = broker.remaining_capacity();
        if available < pending.len() {
            return Err(TaskIngressError::InsufficientCronCapacity {
                required: pending.len(),
                available,
            });
        }
        let first_id = self.reserve_task_ids(pending.len())?;
        let deadline =
            now_ms.checked_add(self.request_timeout_ms).ok_or(TaskIngressError::Clock)?;
        let mut triggered = Vec::with_capacity(pending.len());
        for (offset, (minute, handler, expression, scheduled_at_ms)) in
            pending.into_iter().enumerate()
        {
            let id = TaskId(u128::from(first_id + offset as u64));
            let task = Task::new(
                TaskMeta {
                    id,
                    application_id: self.application_id.clone(),
                    tenant_id: None,
                    idempotency_key: Some(format!("cron:{handler}:{minute}")),
                    trace_id: None,
                },
                TaskTrigger::Cron { name: handler.clone(), expression },
                Some(deadline),
            )
            .with_input(Value::Null);
            broker.enqueue(task)?;
            triggered.push(TriggeredTask { id, handler, scheduled_at_ms });
        }
        *last = Some(current_minute);
        Ok(triggered)
    }

    fn reserve_task_ids(&self, count: usize) -> Result<u64, TaskIngressError> {
        if count == 0 {
            return Ok(self.next_id.load(Ordering::Relaxed));
        }
        let count = u64::try_from(count).map_err(|_| TaskIngressError::TaskIdExhausted)?;
        self.next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |first| {
                first.checked_add(count).filter(|next| *next > first)
            })
            .map_err(|_| TaskIngressError::TaskIdExhausted)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskIngressError {
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),
    #[error("queue '{queue}' has duplicate consumers '{first}' and '{second}'")]
    DuplicateQueueConsumer { queue: String, first: String, second: String },
    #[error("queue '{0}' is not registered")]
    UnknownQueue(String),
    #[error("task id seed must be non-zero")]
    InvalidTaskIdSeed,
    #[error("application id must be non-empty")]
    InvalidApplicationId,
    #[error("task timeout must be non-zero")]
    InvalidTimeout,
    #[error("task id space is exhausted")]
    TaskIdExhausted,
    #[error("task clock overflow")]
    Clock,
    #[error("cron batch requires {required} queue slots but only {available} are available")]
    InsufficientCronCapacity { required: usize, available: usize },
    #[error(transparent)]
    Broker(#[from] TaskRpcBrokerError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_task::TaskState;

    fn definitions() -> Vec<ModuleTaskDefinition> {
        vec![
            ModuleTaskDefinition {
                name: "hourly".into(),
                kind: ModuleTaskKind::Cron { expression: "0 * * * *".into() },
            },
            ModuleTaskDefinition {
                name: "consume-order".into(),
                kind: ModuleTaskKind::Queue { queue: "orders.created".into() },
            },
        ]
    }

    fn ingress(capacity: usize) -> (Arc<Mutex<TaskRpcBroker>>, TaskIngress) {
        let broker = Arc::new(Mutex::new(TaskRpcBroker::new(capacity).unwrap()));
        let registry = TaskRegistry::from_definitions(&definitions()).unwrap();
        let ingress =
            TaskIngress::new(Arc::clone(&broker), registry, "test-app", 10, 5_000).unwrap();
        (broker, ingress)
    }

    #[test]
    fn parses_lists_ranges_steps_and_calendar_days() {
        let schedule = CronExpression::parse("*/15 9-10 1,15 * 1-5").unwrap();
        // 2024-01-15 09:30 UTC, both day fields match.
        assert!(schedule.matches_unix_ms(1_705_311_000_000));
        assert!(!schedule.matches_unix_ms(1_705_311_060_000));
        assert!(CronExpression::parse("0 0 * * 7").unwrap().matches_unix_ms(1_704_585_600_000)); // 2024-01-07 Sunday
    }

    #[test]
    fn rejects_invalid_cron_and_duplicate_queue_consumers() {
        assert!(CronExpression::parse("60 * * * *").is_err());
        assert!(CronExpression::parse("* * *").is_err());
        let mut duplicate = definitions();
        duplicate.push(ModuleTaskDefinition {
            name: "other".into(),
            kind: ModuleTaskKind::Queue { queue: "orders.created".into() },
        });
        assert!(matches!(
            TaskRegistry::from_definitions(&duplicate),
            Err(TaskIngressError::DuplicateQueueConsumer { .. })
        ));
    }

    #[tokio::test]
    async fn queue_submission_routes_input_and_preserves_message_identity() {
        let (broker, ingress) = ingress(2);
        let input = serde_json::json!({"order": 7});
        let id = ingress
            .enqueue_queue("orders.created", Some("message-7".into()), input.clone(), 1_000)
            .await
            .unwrap();
        let broker = broker.lock().await;
        let task = broker.task(id).unwrap();
        assert_eq!(task.state, TaskState::Queued);
        assert_eq!(task.input, input);
        assert_eq!(task.meta.idempotency_key.as_deref(), Some("message-7"));
        assert!(matches!(
            &task.trigger,
            TaskTrigger::Queue { name, handler, message_id }
                if name == "orders.created"
                    && handler == "consume-order"
                    && message_id.as_deref() == Some("message-7")
        ));
    }

    #[tokio::test]
    async fn queue_submission_observes_scheduler_backpressure() {
        let (_, ingress) = ingress(1);
        ingress.enqueue_queue("orders.created", None, Value::Null, 1).await.unwrap();
        assert!(matches!(
            ingress.enqueue_queue("orders.created", None, Value::Null, 2).await,
            Err(TaskIngressError::Broker(TaskRpcBrokerError::Scheduler(
                tysel_scheduler::SchedulerError::Full { capacity: 1 }
            )))
        ));
        assert!(matches!(
            ingress.enqueue_queue("missing", None, Value::Null, 2).await,
            Err(TaskIngressError::UnknownQueue(_))
        ));
    }

    #[tokio::test]
    async fn cron_is_deduplicated_per_minute_and_catches_up() {
        let (broker, ingress) = ingress(4);
        let first = ingress.enqueue_due_cron(3_600_000).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].scheduled_at_ms, 3_600_000);
        assert!(ingress.enqueue_due_cron(3_659_999).await.unwrap().is_empty());
        let caught_up = ingress.enqueue_due_cron(7_200_000).await.unwrap();
        assert_eq!(caught_up.len(), 1);
        assert_eq!(caught_up[0].scheduled_at_ms, 7_200_000);
        assert_eq!(broker.lock().await.task(first[0].id).unwrap().state, TaskState::Queued);
    }

    #[tokio::test]
    async fn cron_does_not_advance_cursor_when_enqueue_hits_backpressure() {
        let (_, ingress) = ingress(1);
        ingress.enqueue_queue("orders.created", None, Value::Null, 1).await.unwrap();
        assert!(ingress.enqueue_due_cron(3_600_000).await.is_err());
        // The same due minute is retried after the original task is claimed.
        let mut worker = ingress.broker.lock().await;
        let claimed = worker.handle(
            1,
            tysel_task_rpc::Envelope::new(tysel_task_rpc::Message::Claim {
                request_id: 1,
                worker_id: "worker".into(),
                lease_ms: 1_000,
                limit: 1,
            }),
        );
        assert!(matches!(claimed.message, tysel_task_rpc::Message::Claimed { .. }));
        drop(worker);
        assert_eq!(ingress.enqueue_due_cron(3_600_000).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cron_batch_is_atomic_when_multiple_handlers_exceed_capacity() {
        let broker = Arc::new(Mutex::new(TaskRpcBroker::new(1).unwrap()));
        let definitions = vec![
            ModuleTaskDefinition {
                name: "first".into(),
                kind: ModuleTaskKind::Cron { expression: "0 * * * *".into() },
            },
            ModuleTaskDefinition {
                name: "second".into(),
                kind: ModuleTaskKind::Cron { expression: "0 * * * *".into() },
            },
        ];
        let registry = TaskRegistry::from_definitions(&definitions).unwrap();
        let ingress =
            TaskIngress::new(Arc::clone(&broker), registry, "test-app", 1, 1_000).unwrap();

        assert!(matches!(
            ingress.enqueue_due_cron(3_600_000).await,
            Err(TaskIngressError::InsufficientCronCapacity { required: 2, available: 1 })
        ));
        assert_eq!(broker.lock().await.remaining_capacity(), 1);
    }
}
