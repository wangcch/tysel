use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use tysel_durable::SqliteStore;
use tysel_durable::{DurableError, DurableStore, ExecutionClaim, WakeupClaim};
use tysel_engine::{EngineError, IsolateConfig, Value};
use tysel_engine_qjs::{DurableCompletion, DurableSession, eval_durable, eval_durable_module};
use tysel_task::TaskId;

const MAX_OWNER_BYTES: usize = 128;

pub struct DurableDispatcher {
    store: Arc<dyn DurableStore>,
    owner: String,
    lease_duration_ms: u64,
    isolate: IsolateConfig,
    pending: Mutex<HashMap<TaskId, PendingCompletion>>,
    slots: Arc<AtomicUsize>,
}

// Bound retained results and in-flight executions together, including HTTP starts.
const MAX_FINALIZATIONS: usize = 32;
pub(crate) struct CompletionSlot(Arc<AtomicUsize>);
impl Drop for CompletionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct PendingCompletion {
    completion: DurableCompletion,
    json: String,
    _slot: CompletionSlot,
}

impl DurableDispatcher {
    pub fn new(
        store: Arc<dyn DurableStore>,
        owner: impl Into<String>,
        lease_duration_ms: u64,
        isolate: IsolateConfig,
    ) -> Result<Self, DispatchError> {
        let owner = owner.into();
        if owner.is_empty() || owner.len() > MAX_OWNER_BYTES {
            return Err(DispatchError::InvalidOwner);
        }
        if lease_duration_ms <= isolate.request_timeout_ms.max(1) {
            return Err(DispatchError::LeaseTooShort {
                lease_duration_ms,
                request_timeout_ms: isolate.request_timeout_ms,
            });
        }
        Ok(Self {
            store,
            owner,
            lease_duration_ms,
            isolate,
            pending: Mutex::new(HashMap::new()),
            slots: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub(crate) fn reserve(&self) -> Result<CompletionSlot, DurableRunError> {
        self.slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_FINALIZATIONS).then_some(n + 1)
            })
            .map_err(|_| DurableRunError::FinalizationCapacity)?;
        Ok(CompletionSlot(self.slots.clone()))
    }

    fn finish(
        &self,
        session: DurableSession,
        value: Value,
        slot: CompletionSlot,
    ) -> Result<DurableRunStatus, DurableRunError> {
        let json = session.take_result().unwrap_or_else(|| value_to_json(&value));
        let completion = session.completion_handle()?;
        drop(session);
        match completion.complete(&json) {
            Ok(_) => Ok(DurableRunStatus::Completed(value)),
            Err(error) => {
                // Retry the retained outcome while this completion authority is valid.
                if !error.is_retryable() {
                    return Err(DurableRunError::boundary(error));
                }
                let json = serde_json::to_string(&json)
                    .map_err(|error| DurableRunError::Session(error.to_string()))?;
                let task_id = completion.task_id();
                tysel_observability::log_durable("completion_pending", Some(task_id.0), 0);
                self.pending
                    .lock()
                    .map_err(|_| DurableError::LockPoisoned)?
                    .insert(task_id, PendingCompletion { completion, json, _slot: slot });
                Err(DurableRunError::Store(error))
            }
        }
    }

    pub(crate) fn has_pending_completions(&self) -> bool {
        self.pending.lock().map_or(true, |pending| !pending.is_empty())
    }

    pub(crate) fn retry_completions(&self) -> (Vec<DurableRun>, Option<DispatchError>) {
        let mut pending = match self.pending.lock() {
            Ok(pending) => pending,
            Err(_) => return (Vec::new(), Some(DurableError::LockPoisoned.into())),
        };
        let ids: Vec<_> = pending.keys().copied().collect();
        let mut runs = Vec::new();
        for task_id in ids {
            let entry = &pending[&task_id];
            let json = serde_json::from_str(&entry.json).expect("validated completion JSON");
            match entry.completion.complete(&json) {
                Ok(_) => {}
                Err(DurableError::ExecutionLeaseLost) => {
                    // This result no longer has write authority. Release its slot;
                    // persisted execution eligibility lets the scheduler reclaim
                    // the original task and replay its recorded boundaries.
                    pending.remove(&task_id);
                    continue;
                }
                Err(error) => return (runs, Some(error.into())),
            }
            pending.remove(&task_id);
            runs.push(DurableRun {
                task_id,
                result: Ok(DurableRunStatus::Completed(value_from_json(json))),
            });
        }
        (runs, None)
    }

    pub(crate) fn store(&self) -> Arc<dyn DurableStore> {
        self.store.clone()
    }

    pub fn start(&self, task_id: TaskId, script: &str) -> DurableRun {
        if self.pending.lock().map_or(true, |pending| pending.contains_key(&task_id)) {
            return DurableRun { task_id, result: Err(DurableRunError::FinalizationPending) };
        }
        match self.store.completion(task_id) {
            Ok(Some(completion)) => {
                return DurableRun {
                    task_id,
                    result: Ok(DurableRunStatus::Completed(value_from_json(completion.value))),
                };
            }
            Err(error) => return DurableRun { task_id, result: Err(error.into()) },
            Ok(None) => {}
        }

        let result = DurableSession::new(self.store.clone(), task_id)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate(script, session));
        DurableRun { task_id, result }
    }

    /// Start an ESM durable task whose default export is
    /// `async (ctx, input) => value`. The input is recorded as the first
    /// durable boundary and is therefore not required when resuming.
    pub fn start_module(&self, task_id: TaskId, source: &str, input_json: &str) -> DurableRun {
        let slot = match self.reserve() {
            Ok(slot) => slot,
            Err(error) => return DurableRun { task_id, result: Err(error) },
        };
        self.start_module_reserved(task_id, source, input_json, slot)
    }

    pub(crate) fn start_module_reserved(
        &self,
        task_id: TaskId,
        source: &str,
        input_json: &str,
        slot: CompletionSlot,
    ) -> DurableRun {
        if self.pending.lock().map_or(true, |pending| pending.contains_key(&task_id)) {
            return DurableRun { task_id, result: Err(DurableRunError::FinalizationPending) };
        }
        match self.store.completion(task_id) {
            Ok(Some(completion)) => {
                return DurableRun {
                    task_id,
                    result: Ok(DurableRunStatus::Completed(value_from_json(completion.value))),
                };
            }
            Err(error) => return DurableRun { task_id, result: Err(error.into()) },
            Ok(None) => {}
        }

        let result = DurableSession::new(self.store.clone(), task_id)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate_module(source, input_json, session, slot));
        DurableRun { task_id, result }
    }

    pub(crate) fn start_admitted(
        &self,
        task_id: TaskId,
        source: &str,
        slot: CompletionSlot,
    ) -> Result<Option<DurableRun>, DispatchError> {
        if let Some(completion) = self.store.completion(task_id)? {
            return Ok(Some(DurableRun {
                task_id,
                result: Ok(DurableRunStatus::Completed(value_from_json(completion.value))),
            }));
        }
        let Some(claim) =
            self.store.claim_execution(task_id, &self.owner, self.lease_duration_ms)?
        else {
            if self.store.execution_failed(task_id)? {
                return Ok(Some(DurableRun { task_id, result: Err(DurableRunError::TaskFailed) }));
            }
            return Ok(None);
        };
        Ok(Some(self.execute_managed(claim, source, true, slot)))
    }

    fn execute_managed(
        &self,
        claim: ExecutionClaim,
        source: &str,
        module: bool,
        slot: CompletionSlot,
    ) -> DurableRun {
        let task_id = claim.task_id;
        let result = DurableSession::from_execution(self.store.clone(), &claim)
            .map_err(DurableRunError::Session)
            .and_then(|session| {
                if module {
                    self.evaluate_module(source, "null", session, slot)
                } else {
                    let result = eval_durable(source, self.isolate, session.clone());
                    if let Some(error) = session.take_storage_error() {
                        return Err(DurableRunError::boundary(error));
                    }
                    match result {
                        Ok(value) => self.finish(session, value, slot),
                        Err(EngineError::Suspended) => Ok(DurableRunStatus::Suspended),
                        Err(error) => Err(DurableRunError::Engine(error)),
                    }
                }
            });
        let transition = match &result {
            Ok(DurableRunStatus::Suspended) => self.store.finish_execution(&claim, false),
            Err(
                DurableRunError::Store(_)
                | DurableRunError::Session(_)
                | DurableRunError::LeaseLost,
            ) => Ok(()), // retained completion retries retain this lease
            Err(_) => self.store.finish_execution(&claim, true),
            _ => Ok(()),
        };
        DurableRun {
            task_id,
            result: match transition {
                Ok(()) => result,
                Err(error) => Err(DurableRunError::boundary(error)),
            },
        }
    }

    /// Claim and execute one exact registered task if its wakeup is due.
    pub fn dispatch_task(
        &self,
        task_id: TaskId,
        script: &str,
    ) -> Result<Option<DurableRun>, DispatchError> {
        // Caller-owned in-memory registries retain their low-level wakeup contract.
        if self.store.program(task_id)?.is_none() {
            let Some(claim) = self.store.claim_wakeup(
                task_id,
                unix_time_ms()?,
                &self.owner,
                self.lease_duration_ms,
            )?
            else {
                return Ok(None);
            };
            return Ok(Some(self.execute_claim(claim, script)));
        }
        let Ok(slot) = self.reserve() else {
            return Ok(None);
        };
        let Some(claim) =
            self.store.claim_execution(task_id, &self.owner, self.lease_duration_ms)?
        else {
            return Ok(None);
        };
        Ok(Some(self.execute_managed(claim, script, false, slot)))
    }

    pub fn dispatch_module_task(
        &self,
        task_id: TaskId,
        source: &str,
    ) -> Result<Option<DurableRun>, DispatchError> {
        let Ok(slot) = self.reserve() else {
            return Ok(None);
        };
        let Some(claim) =
            self.store.claim_execution(task_id, &self.owner, self.lease_duration_ms)?
        else {
            return Ok(None);
        };
        Ok(Some(self.execute_managed(claim, source, true, slot)))
    }

    /// Claim and execute up to `limit` due tasks. The resolver supplies the
    /// persisted program for each task id; unresolved claims are safely released.
    pub fn dispatch_due<F, S>(
        &self,
        limit: usize,
        mut resolve: F,
    ) -> Result<Vec<DurableRun>, DispatchError>
    where
        F: FnMut(TaskId) -> Option<S>,
        S: AsRef<str>,
    {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now_ms = unix_time_ms()?;
        let claims =
            self.store.claim_due_wakeups(now_ms, limit, &self.owner, self.lease_duration_ms)?;
        let mut runs = Vec::with_capacity(claims.len());
        for claim in claims {
            let task_id = claim.task_id;
            let Some(script) = resolve(task_id) else {
                let released = self.store.release_wakeup_claim(&claim)?;
                runs.push(DurableRun {
                    task_id,
                    result: Err(if released {
                        DurableRunError::ProgramNotFound
                    } else {
                        DurableRunError::LeaseLost
                    }),
                });
                continue;
            };
            let now_ms = unix_time_ms()?;
            let Some(claim) =
                self.store.renew_wakeup_claim(&claim, now_ms, self.lease_duration_ms)?
            else {
                runs.push(DurableRun { task_id, result: Err(DurableRunError::LeaseLost) });
                continue;
            };
            runs.push(self.execute_claim(claim, script.as_ref()));
        }
        Ok(runs)
    }

    fn execute_claim(&self, claim: WakeupClaim, script: &str) -> DurableRun {
        let task_id = claim.task_id;
        let result = DurableSession::from_claim(self.store.clone(), claim)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate(script, session));
        DurableRun { task_id, result }
    }

    fn evaluate(
        &self,
        script: &str,
        session: DurableSession,
    ) -> Result<DurableRunStatus, DurableRunError> {
        let slot = self.reserve()?;
        let result = eval_durable(script, self.isolate, session.clone());
        if let Some(error) = session.take_storage_error() {
            return Err(DurableRunError::boundary(error));
        }
        match result {
            Ok(value) => {
                if matches!(&value, Value::Number(number) if !number.is_finite()) {
                    return Err(DurableRunError::Engine(EngineError::Isolate(
                        "durable result must be finite JSON".into(),
                    )));
                }
                self.finish(session, value, slot)
            }
            Err(EngineError::Suspended) => Ok(DurableRunStatus::Suspended),
            Err(error) => Err(DurableRunError::Engine(error)),
        }
    }

    fn evaluate_module(
        &self,
        source: &str,
        input_json: &str,
        session: DurableSession,
        slot: CompletionSlot,
    ) -> Result<DurableRunStatus, DurableRunError> {
        let result = eval_durable_module(source, input_json, self.isolate, session.clone());
        if let Some(error) = session.take_storage_error() {
            return Err(DurableRunError::boundary(error));
        }
        match result {
            Ok(value) => self.finish(session, value, slot),
            Err(EngineError::Suspended) => Ok(DurableRunStatus::Suspended),
            Err(error) => Err(DurableRunError::Engine(error)),
        }
    }
}

pub(crate) fn value_to_json(value: &Value) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Number(value) => {
            serde_json::Number::from_f64(*value).map(Json::Number).unwrap_or(Json::Null)
        }
        Value::String(value) => Json::String(value.clone()),
        Value::Bytes(value) => Json::Array(value.iter().copied().map(Json::from).collect()),
        Value::Array(values) => Json::Array(values.iter().map(value_to_json).collect()),
        Value::Record(values) => Json::Object(
            values.iter().map(|(key, value)| (key.clone(), value_to_json(value))).collect(),
        ),
    }
}

fn value_from_json(value: serde_json::Value) -> Value {
    use serde_json::Value as Json;
    match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(value),
        Json::Number(value) => Value::Number(value.as_f64().unwrap_or_default()),
        Json::String(value) => Value::String(value),
        Json::Array(values) => Value::Array(values.into_iter().map(value_from_json).collect()),
        Json::Object(values) => Value::Record(
            values.into_iter().map(|(key, value)| (key, value_from_json(value))).collect(),
        ),
    }
}

#[derive(Debug)]
pub struct DurableRun {
    pub task_id: TaskId,
    pub result: Result<DurableRunStatus, DurableRunError>,
}

#[derive(Debug, PartialEq)]
pub enum DurableRunStatus {
    Completed(Value),
    Suspended,
}

#[derive(Debug, thiserror::Error)]
pub enum DurableRunError {
    #[error("durable task has already failed; the same admission will not be executed again")]
    TaskFailed,
    #[error("durable task boundary: {0}")]
    Boundary(#[source] DurableError),
    #[error("durable finalization capacity exhausted")]
    FinalizationCapacity,
    #[error("durable completion is pending persistence")]
    FinalizationPending,
    #[error("durable session: {0}")]
    Session(String),
    #[error("durable engine: {0}")]
    Engine(#[source] EngineError),
    #[error(transparent)]
    Store(#[from] DurableError),
    #[error("durable task program was not found")]
    ProgramNotFound,
    #[error("durable wakeup lease was lost")]
    LeaseLost,
}

impl DurableRunError {
    fn boundary(error: DurableError) -> Self {
        match error {
            DurableError::ExecutionLeaseLost => Self::LeaseLost,
            DurableError::EventKeyTooLarge
            | DurableError::EventPayloadTooLarge
            | DurableError::HistoryEventLimit
            | DurableError::HistoryByteLimit
            | DurableError::InvalidSignalName
            | DurableError::SignalWaitConflict
            | DurableError::SignalInboxLimit
            | DurableError::InvalidEventKind(_)
            | DurableError::IntegerRange { .. }
            | DurableError::Json(_) => Self::Boundary(error),
            _ => Self::Store(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("dispatcher owner must be 1..={MAX_OWNER_BYTES} bytes")]
    InvalidOwner,
    #[error(
        "wakeup lease {lease_duration_ms}ms must exceed request timeout {request_timeout_ms}ms"
    )]
    LeaseTooShort { lease_duration_ms: u64, request_timeout_ms: u64 },
    #[error(transparent)]
    Store(#[from] DurableError),
    #[error("system clock is before the Unix epoch")]
    SystemClock,
    #[error("system time is too large")]
    TimeRange,
}

fn unix_time_ms() -> Result<u64, DispatchError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DispatchError::SystemClock)?
        .as_millis();
    u64::try_from(millis).map_err(|_| DispatchError::TimeRange)
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::time::Duration;

    use super::*;
    use tysel_engine::InterruptReason;

    const WAIT_SCRIPT: &str = r#"
        (async () => JSON.stringify(await tysel.durable.waitForSignal("approval")))()
    "#;

    fn config() -> IsolateConfig {
        IsolateConfig {
            request_timeout_ms: 500,
            cpu_ms_per_turn: 50,
            memory_limit_bytes: 8 * 1024 * 1024,
        }
    }

    fn dispatcher(store: Arc<SqliteStore>, owner: &str) -> DurableDispatcher {
        DurableDispatcher::new(store, owner, 1_000, config()).unwrap()
    }

    #[test]
    fn expired_pending_completion_releases_slot_and_recovers_original_history() {
        let path =
            std::env::temp_dir().join(format!("tysel-pending-expiry-{}.db", std::process::id()));
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();
        let source = "export default async ctx => await ctx.effect('write',async()=>42)";
        let id = TaskId(910);
        store.admit_module(id, "pending-expiry", source, "null", 0).unwrap();
        db.execute_batch("CREATE TRIGGER fail_completion BEFORE INSERT ON durable_completions BEGIN SELECT * FROM missing_completion_table; END;").unwrap();
        let dispatcher = dispatcher(store.clone(), "pending-expiry");
        let run =
            dispatcher.start_admitted(id, source, dispatcher.reserve().unwrap()).unwrap().unwrap();
        assert!(matches!(run.result, Err(DurableRunError::Store(_))));
        assert!(dispatcher.has_pending_completions());
        assert_eq!(store.load_history(id).unwrap().events.len(), 2);
        std::thread::sleep(Duration::from_millis(1100));
        let (runs, error) = dispatcher.retry_completions();
        assert!(error.is_none(), "{error:?}");
        assert!(runs.is_empty(), "an expired result must not notify completion");
        assert!(!dispatcher.has_pending_completions(), "expired handle must release its slot");
        assert!(store.completion(id).unwrap().is_none());
        db.execute_batch("DROP TRIGGER fail_completion").unwrap();
        let run = dispatcher.dispatch_module_task(id, source).unwrap().unwrap();
        assert!(matches!(run.result, Ok(DurableRunStatus::Completed(Value::Number(42.0)))));
        assert_eq!(store.load_history(id).unwrap().events.len(), 2);
        assert_eq!(store.completion(id).unwrap().unwrap().value, serde_json::json!(42));
        drop(dispatcher);
        drop(db);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn managed_storage_failure_recovers_even_when_javascript_catches_it() {
        for caught in [false, true] {
            let path = std::env::temp_dir()
                .join(format!("tysel-storage-fault-{}-{caught}.db", std::process::id()));
            let store = Arc::new(SqliteStore::open(&path).unwrap());
            let db = rusqlite::Connection::open(&path).unwrap();
            let source = if caught {
                "export default async ctx => {try {return await ctx.effect('write',async()=>42)} catch {return 99}}"
            } else {
                "export default async ctx => await ctx.effect('write',async()=>42)"
            };
            let id = TaskId(900);
            store.admit_module(id, "fault", source, "null", 0).unwrap();
            db.execute_batch("CREATE TRIGGER fail_effect BEFORE INSERT ON durable_events WHEN NEW.sequence=1 BEGIN SELECT RAISE(FAIL, 'temporary storage fault'); END;").unwrap();
            let dispatcher = dispatcher(store.clone(), "fault-test");
            let run = dispatcher
                .start_admitted(id, source, dispatcher.reserve().unwrap())
                .unwrap()
                .unwrap();
            assert!(matches!(run.result, Err(DurableRunError::Store(_))), "{run:?}");
            assert!(
                store.completion(id).unwrap().is_none(),
                "caught error must not commit fallback"
            );
            assert_eq!(
                db.query_row("SELECT state FROM durable_executions", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "running"
            );
            db.execute_batch("DROP TRIGGER fail_effect").unwrap();
            std::thread::sleep(Duration::from_millis(1100));
            let due = store
                .load_due_programs_batch(
                    unix_time_ms().unwrap(),
                    tysel_durable::DurableProgramKind::Module,
                    32,
                )
                .unwrap();
            assert_eq!(due.len(), 1);
            let run = dispatcher.dispatch_module_task(id, source).unwrap().unwrap();
            assert!(
                matches!(run.result, Ok(DurableRunStatus::Completed(Value::Number(42.0)))),
                "{run:?}"
            );
            assert_eq!(store.completion(id).unwrap().unwrap().value, serde_json::json!(42));
            drop(dispatcher);
            drop(db);
            drop(store);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn managed_business_exception_remains_failed() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let source = "export default async () => {throw new Error('business failure')}";
        let id = TaskId(901);
        store.admit_module(id, "business", source, "null", 0).unwrap();
        let dispatcher = dispatcher(store.clone(), "business-test");
        let run =
            dispatcher.start_admitted(id, source, dispatcher.reserve().unwrap()).unwrap().unwrap();
        assert!(matches!(run.result, Err(DurableRunError::Engine(_))));
        assert!(store.claim_execution(id, "other", 1000).unwrap().is_none());
        let retry =
            dispatcher.start_admitted(id, source, dispatcher.reserve().unwrap()).unwrap().unwrap();
        assert!(matches!(retry.result, Err(DurableRunError::TaskFailed)));
        assert!(
            store
                .load_due_programs_batch(
                    unix_time_ms().unwrap() + 2000,
                    tysel_durable::DurableProgramKind::Module,
                    32
                )
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn non_finite_script_result_is_not_silently_persisted_as_null() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(801);
        store.put_program(id, "Infinity", 0).unwrap();
        assert!(dispatcher(store.clone(), "runner").start(id, "Infinity").result.is_err());
        assert!(store.completion(id).unwrap().is_none());
    }

    #[test]
    fn integer_array_completion_keeps_the_validated_json_size() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(802);
        let source = "export default async () => Array(300000).fill(0)";
        store.put_module(id, source, 0).unwrap();
        let dispatcher = DurableDispatcher::new(
            store.clone(),
            "large-result",
            15_000,
            IsolateConfig {
                memory_limit_bytes: 256 * 1024 * 1024,
                cpu_ms_per_turn: 4_000,
                request_timeout_ms: 10_000,
            },
        )
        .unwrap();
        let run = dispatcher.start_module(id, source, "null");
        assert!(matches!(run.result, Ok(DurableRunStatus::Completed(_))), "{run:?}");
        let json = serde_json::to_string(&store.completion(id).unwrap().unwrap().value).unwrap();
        assert_eq!(json.len(), 600_001);
        assert_eq!(dispatcher.slots.load(Ordering::Acquire), 0);
    }

    #[test]
    fn finalization_slots_are_bounded_and_released() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "bounded");
        let slots: Vec<_> = (0..MAX_FINALIZATIONS).map(|_| dispatcher.reserve().unwrap()).collect();
        assert!(matches!(dispatcher.reserve(), Err(DurableRunError::FinalizationCapacity)));
        drop(slots);
        assert!(dispatcher.reserve().is_ok());
    }

    #[test]
    fn persisted_completion_returns_result_without_reexecuting() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(800);
        let source = "export default async (ctx, input) => ({answer: input.answer})";
        store.put_module(id, source, 0).unwrap();
        let dispatcher = dispatcher(store.clone(), "runner");
        let first = dispatcher.start_module(id, source, r#"{"answer":42}"#);
        assert!(matches!(first.result, Ok(DurableRunStatus::Completed(_))));
        let cached = dispatcher.start_module(id, "throw new Error('must not execute')", "null");
        match (first.result.unwrap(), cached.result.unwrap()) {
            (DurableRunStatus::Completed(first), DurableRunStatus::Completed(cached)) => {
                assert_eq!(first, cached)
            }
            _ => panic!("expected retained completion"),
        }
        assert_eq!(store.program_count().unwrap(), 0);
        assert_eq!(store.load_history(id).unwrap().events.len(), 1);
    }

    #[test]
    fn starts_suspends_and_resumes_a_signal_task() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "runner-a");
        let id = TaskId(201);
        assert!(matches!(
            dispatcher.start(id, WAIT_SCRIPT).result,
            Ok(DurableRunStatus::Suspended)
        ));

        let now_ms = unix_time_ms().unwrap();
        store.send_signal(id, "approval", &serde_json::json!({"ok": true}), now_ms).unwrap();
        let runs =
            dispatcher.dispatch_due(10, |task_id| (task_id == id).then_some(WAIT_SCRIPT)).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].result.as_ref().unwrap(),
            &DurableRunStatus::Completed(Value::String(r#"{"ok":true}"#.into()))
        );
        assert_eq!(store.wakeup(id).unwrap(), None);
        assert_eq!(store.signal_wait(id).unwrap(), None);
    }

    #[test]
    fn resumes_a_due_sleep_task() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "runner-a");
        let id = TaskId(206);
        let script = r#"(async () => { await tysel.durable.sleep("30ms"); return "awake"; })()"#;
        assert!(matches!(dispatcher.start(id, script).result, Ok(DurableRunStatus::Suspended)));
        let wakeup = store.wakeup(id).unwrap().unwrap();
        let remaining = wakeup.wake_at_ms.saturating_sub(unix_time_ms().unwrap());
        std::thread::sleep(Duration::from_millis(remaining + 1));

        let runs = dispatcher.dispatch_due(1, |_| Some(script)).unwrap();
        assert_eq!(
            runs[0].result.as_ref().unwrap(),
            &DurableRunStatus::Completed(Value::String("awake".into()))
        );
        assert_eq!(store.wakeup(id).unwrap(), None);
    }

    #[test]
    fn resumes_a_retry_after_its_durable_backoff() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "runner-a");
        let id = TaskId(208);
        let script = r#"
            (async () => tysel.durable.retry(
                { maxAttempts: 2, delay: "30ms" },
                (attempt) => {
                    if (attempt === 1) throw new Error("retry me");
                    return attempt;
                },
            ))()
        "#;
        assert!(matches!(dispatcher.start(id, script).result, Ok(DurableRunStatus::Suspended)));
        let wakeup = store.wakeup(id).unwrap().unwrap();
        let remaining = wakeup.wake_at_ms.saturating_sub(unix_time_ms().unwrap());
        std::thread::sleep(Duration::from_millis(remaining + 1));

        let runs = dispatcher.dispatch_due(1, |_| Some(script)).unwrap();
        assert_eq!(
            runs[0].result.as_ref().unwrap(),
            &DurableRunStatus::Completed(Value::Number(2.0))
        );
        assert_eq!(store.wakeup(id).unwrap(), None);
    }

    #[test]
    fn missing_program_releases_the_claim_for_another_dispatcher() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let first = dispatcher(store.clone(), "runner-a");
        let second = dispatcher(store.clone(), "runner-b");
        let id = TaskId(202);
        assert!(matches!(first.start(id, WAIT_SCRIPT).result, Ok(DurableRunStatus::Suspended)));
        store
            .send_signal(id, "approval", &serde_json::json!("yes"), unix_time_ms().unwrap())
            .unwrap();

        let missing = first.dispatch_due(1, |_| None::<&str>).unwrap();
        assert!(matches!(missing[0].result, Err(DurableRunError::ProgramNotFound)));
        let resumed = second.dispatch_due(1, |_| Some(WAIT_SCRIPT)).unwrap();
        assert!(matches!(resumed[0].result, Ok(DurableRunStatus::Completed(_))));
    }

    #[test]
    fn concurrent_dispatchers_execute_one_wakeup_once() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let setup = dispatcher(store.clone(), "setup-runner");
        let id = TaskId(205);
        assert!(matches!(setup.start(id, WAIT_SCRIPT).result, Ok(DurableRunStatus::Suspended)));
        store
            .send_signal(id, "approval", &serde_json::json!("yes"), unix_time_ms().unwrap())
            .unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = [
            Arc::new(dispatcher(store.clone(), "runner-a")),
            Arc::new(dispatcher(store.clone(), "runner-b")),
        ]
        .into_iter()
        .map(|dispatcher| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                dispatcher.dispatch_due(1, |_| Some(WAIT_SCRIPT)).unwrap()
            })
        })
        .collect();
        barrier.wait();
        let runs: Vec<_> = threads.into_iter().flat_map(|thread| thread.join().unwrap()).collect();
        assert_eq!(runs.len(), 1);
        assert!(matches!(runs[0].result, Ok(DurableRunStatus::Completed(_))));
        assert_eq!(store.load_history(id).unwrap().events.len(), 1);
    }

    #[test]
    fn changed_program_cannot_skip_a_claimed_boundary() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "runner-a");
        let id = TaskId(203);
        assert!(matches!(
            dispatcher.start(id, WAIT_SCRIPT).result,
            Ok(DurableRunStatus::Suspended)
        ));
        store
            .send_signal(id, "approval", &serde_json::json!(true), unix_time_ms().unwrap())
            .unwrap();

        let runs = dispatcher.dispatch_due(1, |_| Some("42")).unwrap();
        assert!(matches!(
            &runs[0].result,
            Err(DurableRunError::Engine(EngineError::Isolate(message)))
                if message.contains("persisted suspension")
        ));
        assert!(store.wakeup(id).unwrap().is_some());
    }

    #[test]
    fn ordinary_timeout_is_not_reported_as_a_suspension() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store, "runner-a");
        let run = dispatcher.start(TaskId(204), "(async () => tysel.sleep(1000))()");
        assert!(matches!(
            run.result,
            Err(DurableRunError::Engine(EngineError::Interrupted(InterruptReason::Timeout)))
        ));
    }

    #[test]
    fn engine_failure_after_starting_a_wait_is_not_reported_as_suspended() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store, "runner-a");
        let run = dispatcher.start(
            TaskId(207),
            r#"(async () => { tysel.durable.waitForSignal("approval"); throw new Error("boom"); })()"#,
        );
        assert!(matches!(
            run.result,
            Err(DurableRunError::Engine(EngineError::Isolate(message)))
                if message.contains("boom")
        ));
    }

    #[test]
    fn lease_must_outlive_one_execution_deadline() {
        assert!(matches!(
            DurableDispatcher::new(
                Arc::new(SqliteStore::in_memory().unwrap()),
                "runner",
                10,
                config()
            ),
            Err(DispatchError::LeaseTooShort { .. })
        ));
    }
}
