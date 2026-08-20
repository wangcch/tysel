use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tysel_durable::{DurableError, SqliteStore, WakeupClaim};
use tysel_engine::{EngineError, IsolateConfig, Value};
use tysel_engine_qjs::{DurableSession, eval_durable, eval_durable_module};
use tysel_task::TaskId;

const MAX_OWNER_BYTES: usize = 128;

pub struct DurableDispatcher {
    store: Arc<SqliteStore>,
    owner: String,
    lease_duration_ms: u64,
    isolate: IsolateConfig,
}

impl DurableDispatcher {
    pub fn new(
        store: Arc<SqliteStore>,
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
        Ok(Self { store, owner, lease_duration_ms, isolate })
    }

    pub(crate) fn store(&self) -> Arc<SqliteStore> {
        self.store.clone()
    }

    pub fn start(&self, task_id: TaskId, script: &str) -> DurableRun {
        let result = DurableSession::new(self.store.clone(), task_id)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate(script, session));
        DurableRun { task_id, result }
    }

    /// Start an ESM durable task whose default export is
    /// `async (ctx, input) => value`. The input is recorded as the first
    /// durable boundary and is therefore not required when resuming.
    pub fn start_module(&self, task_id: TaskId, source: &str, input_json: &str) -> DurableRun {
        let result = DurableSession::new(self.store.clone(), task_id)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate_module(source, input_json, session));
        DurableRun { task_id, result }
    }

    /// Claim and execute one exact registered task if its wakeup is due.
    pub fn dispatch_task(
        &self,
        task_id: TaskId,
        script: &str,
    ) -> Result<Option<DurableRun>, DispatchError> {
        let Some(claim) = self.store.claim_wakeup(
            task_id,
            unix_time_ms()?,
            &self.owner,
            self.lease_duration_ms,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(self.execute_claim(claim, script)))
    }

    pub fn dispatch_module_task(
        &self,
        task_id: TaskId,
        source: &str,
    ) -> Result<Option<DurableRun>, DispatchError> {
        let Some(claim) = self.store.claim_wakeup(
            task_id,
            unix_time_ms()?,
            &self.owner,
            self.lease_duration_ms,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(self.execute_module_claim(claim, source)))
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

    fn execute_module_claim(&self, claim: WakeupClaim, source: &str) -> DurableRun {
        let task_id = claim.task_id;
        let result = DurableSession::from_claim(self.store.clone(), claim)
            .map_err(DurableRunError::Session)
            .and_then(|session| self.evaluate_module(source, "null", session));
        DurableRun { task_id, result }
    }

    fn evaluate(
        &self,
        script: &str,
        session: DurableSession,
    ) -> Result<DurableRunStatus, DurableRunError> {
        match eval_durable(script, self.isolate, session) {
            Ok(value) => Ok(DurableRunStatus::Completed(value)),
            Err(EngineError::Suspended) => Ok(DurableRunStatus::Suspended),
            Err(error) => Err(DurableRunError::Engine(error)),
        }
    }

    fn evaluate_module(
        &self,
        source: &str,
        input_json: &str,
        session: DurableSession,
    ) -> Result<DurableRunStatus, DurableRunError> {
        match eval_durable_module(source, input_json, self.isolate, session) {
            Ok(value) => Ok(DurableRunStatus::Completed(value)),
            Err(EngineError::Suspended) => Ok(DurableRunStatus::Suspended),
            Err(error) => Err(DurableRunError::Engine(error)),
        }
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
                if message.contains("Exception generated by QuickJS")
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
