use std::sync::{Arc, Mutex};

use serde_json::json;
use tysel_durable::{
    DurableError, DurableStore, EventKind, ExecutionClaim, ReplayCursor, ReplayError, WakeupClaim,
};
use tysel_task::TaskId;

#[derive(Clone)]
pub struct DurableSession {
    inner: Arc<Mutex<SessionInner>>,
}

struct SessionInner {
    store: Arc<dyn DurableStore>,
    task_id: TaskId,
    replay: ReplayCursor,
    next_sequence: u64,
    active_wakeup: Option<WakeupToken>,
    suspended: bool,
    lease_until_ms: Option<u64>,
    result_json: Option<serde_json::Value>,
    replayed_sleep: Option<u64>,
    storage_error: Option<DurableError>,
}

impl SessionInner {
    fn check_storage(&self) -> Result<(), String> {
        match &self.storage_error {
            Some(error) => Err(error.to_string()),
            None => Ok(()),
        }
    }

    fn capture_storage(&mut self, error: DurableError) -> String {
        let message = error.to_string();
        if self.storage_error.is_none() {
            self.storage_error = Some(error);
        }
        message
    }
}

struct WakeupToken {
    sequence: u64,
    kind: WakeupKind,
    claim: Option<WakeupClaim>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WakeupKind {
    Sleep,
    Signal,
}

impl DurableSession {
    pub(crate) fn record_input_json(&self, input_json: &str) -> Result<String, String> {
        const INPUT_KEY: &str = "$tysel:task-input";
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        if let Some(event) =
            inner.replay.consume_event(EventKind::Step, INPUT_KEY).map_err(replay_error)?
        {
            return Ok(event.payload_json().into());
        }
        let stored = inner
            .store
            .append_event_json_at(
                inner.task_id,
                inner.next_sequence,
                EventKind::Step,
                INPUT_KEY.into(),
                input_json,
                unix_time_ms()?,
            )
            .map_err(|err| inner.capture_storage(err))?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        Ok(stored.payload_json().into())
    }

    /// Start a task that has no pending wakeup. Suspended tasks must enter via
    /// `from_claim` so they cannot resume early or run under two schedulers.
    pub fn new(store: Arc<dyn DurableStore>, task_id: TaskId) -> Result<Self, String> {
        if store.completion(task_id).map_err(|err| err.to_string())?.is_some() {
            return Err("durable task is already completed".into());
        }
        if let Some(wakeup) = store.wakeup(task_id).map_err(|err| err.to_string())? {
            return Err(format!(
                "durable task is suspended until {} and must be resumed from a wakeup claim",
                wakeup.wake_at_ms
            ));
        }
        if let Some(wait) = store.signal_wait(task_id).map_err(|err| err.to_string())? {
            return Err(format!(
                "durable task is suspended waiting for signal {:?}",
                wait.signal_name
            ));
        }
        Self::load(store, task_id, None)
    }

    pub fn from_claim(store: Arc<dyn DurableStore>, claim: WakeupClaim) -> Result<Self, String> {
        let now_ms = unix_time_ms()?;
        if now_ms < claim.wake_at_ms {
            return Err(format!("durable wakeup is not due until {}", claim.wake_at_ms));
        }
        if !store.claim_is_active(&claim, now_ms).map_err(|err| err.to_string())? {
            return Err("durable wakeup claim is missing or expired".into());
        }
        let task_id = claim.task_id;
        Self::load(store, task_id, Some(claim))
    }

    pub fn from_execution(
        store: Arc<dyn DurableStore>,
        claim: &ExecutionClaim,
    ) -> Result<Self, String> {
        let scoped = store.execution_store(claim);
        let session = if let Some(wakeup) = &claim.wakeup {
            Self::from_claim(scoped, wakeup.clone())?
        } else {
            Self::load(scoped, claim.task_id, None)?
        };
        session.inner.lock().map_err(|_| "durable session lock")?.lease_until_ms =
            Some(claim.lease_until_ms);
        Ok(session)
    }

    fn load(
        store: Arc<dyn DurableStore>,
        task_id: TaskId,
        active_claim: Option<WakeupClaim>,
    ) -> Result<Self, String> {
        let lease_until_ms = active_claim.as_ref().map(|claim| claim.lease_until_ms);
        let history = store.load_history(task_id).map_err(|err| err.to_string())?;
        let next_sequence = history
            .events
            .last()
            .map(|event| event.sequence.checked_add(1).ok_or("durable history is too large"))
            .transpose()?
            .unwrap_or(0);
        let active_wakeup = if let Some(claim) = active_claim {
            let kind = if next_sequence == claim.sequence.saturating_add(1)
                && history.events.last().is_some_and(|event| {
                    event.sequence == claim.sequence && event.kind == EventKind::Sleep
                }) {
                WakeupKind::Sleep
            } else if next_sequence == claim.sequence
                && store
                    .signal_wait(task_id)
                    .map_err(|err| err.to_string())?
                    .is_some_and(|wait| wait.sequence == claim.sequence)
            {
                WakeupKind::Signal
            } else {
                return Err("durable wakeup does not reference a suspended boundary".into());
            };
            Some(WakeupToken { sequence: claim.sequence, kind, claim: Some(claim) })
        } else {
            None
        };
        let replay = history.replay();
        Ok(Self {
            inner: Arc::new(Mutex::new(SessionInner {
                store,
                task_id,
                replay,
                next_sequence,
                active_wakeup,
                suspended: false,
                lease_until_ms,
                result_json: None,
                replayed_sleep: None,
                storage_error: None,
            })),
        })
    }

    pub(crate) fn lookup_json(&self, kind: &str, key: &str) -> Result<String, String> {
        let kind = parse_kind(kind)?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        let event = inner.replay.consume_event(kind, key).map_err(replay_error)?.cloned();
        inner.replayed_sleep = if kind == EventKind::Sleep {
            event.as_ref().map(|event| event.sequence)
        } else {
            None
        };
        Ok(match event {
            Some(event) => format!(
                r#"{{"found":true,"payload":{},"sequence":{},"recordedAtMs":{}}}"#,
                event.payload_json(),
                event.sequence,
                event.recorded_at_ms,
            ),
            None => r#"{"found":false}"#.into(),
        })
    }

    pub(crate) fn find_retry_outcome_json(&self, key: &str) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        let event = inner.replay.consume_through(EventKind::Retry, key).cloned();
        Ok(match event {
            Some(event) => format!(
                r#"{{"found":true,"payload":{},"sequence":{},"recordedAtMs":{}}}"#,
                event.payload_json(),
                event.sequence,
                event.recorded_at_ms,
            ),
            None => r#"{"found":false}"#.into(),
        })
    }

    pub(crate) fn record(
        &self,
        kind: &str,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
    ) -> Result<(), String> {
        let kind = parse_kind(kind)?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        let stored = inner
            .store
            .append_event_json_at(
                inner.task_id,
                inner.next_sequence,
                kind,
                key,
                payload_json,
                recorded_at_ms,
            )
            .map_err(|err| inner.capture_storage(err))?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        Ok(())
    }

    pub(crate) fn record_sleep(
        &self,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
        wake_at_ms: u64,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        let stored = inner
            .store
            .append_event_json_with_wakeup_at(
                inner.task_id,
                inner.next_sequence,
                key,
                payload_json,
                recorded_at_ms,
                wake_at_ms,
            )
            .map_err(|err| inner.capture_storage(err))?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        inner.active_wakeup =
            Some(WakeupToken { sequence: stored.sequence, kind: WakeupKind::Sleep, claim: None });
        inner.suspended = true;
        Ok(())
    }

    pub(crate) fn complete_sleep(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        if let Some(sequence) = inner.replayed_sleep.take()
            && !inner
                .active_wakeup
                .as_ref()
                .is_some_and(|token| token.kind == WakeupKind::Sleep && token.sequence == sequence)
        {
            return Ok(());
        }

        let token = inner
            .active_wakeup
            .as_ref()
            .ok_or_else(|| "durable sleep has no active wakeup claim".to_string())?;
        if token.kind != WakeupKind::Sleep {
            return Err("active durable wakeup belongs to a signal wait".into());
        }
        let completed = inner
            .store
            .complete_wakeup(
                inner.task_id,
                token.sequence,
                token.claim.as_ref().map(|claim| claim.lease_owner.as_str()),
                unix_time_ms()?,
            )
            .map_err(|err| inner.capture_storage(err))?;
        if !completed {
            return Err("durable wakeup ownership was lost".into());
        }
        inner.active_wakeup = None;
        inner.suspended = false;
        Ok(())
    }

    pub(crate) fn poll_signal_json(&self, signal_name: &str) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        let claim = inner
            .active_wakeup
            .as_ref()
            .filter(|token| token.kind == WakeupKind::Signal)
            .and_then(|token| token.claim.as_ref());
        let event = inner
            .store
            .poll_signal(inner.task_id, inner.next_sequence, signal_name, unix_time_ms()?, claim)
            .map_err(|err| inner.capture_storage(err))?;
        let response = if let Some(event) = event {
            inner.next_sequence = event.sequence.saturating_add(1);
            if inner.active_wakeup.as_ref().is_some_and(|token| {
                token.kind == WakeupKind::Signal && token.sequence == event.sequence
            }) {
                inner.active_wakeup = None;
            }
            inner.suspended = false;
            json!({ "found": true, "payload": event.payload })
        } else {
            inner.suspended = true;
            json!({ "found": false })
        };
        serde_json::to_string(&response).map_err(|err| err.to_string())
    }

    /// Preserve host storage failures across the JavaScript exception boundary.
    /// The dispatcher checks this even when user code catches the exception.
    pub fn take_storage_error(&self) -> Option<DurableError> {
        match self.inner.lock() {
            Ok(mut inner) => inner.storage_error.take(),
            Err(_) => Some(DurableError::LockPoisoned),
        }
    }

    pub fn task_id(&self) -> TaskId {
        self.inner.lock().expect("durable session lock").task_id
    }

    pub(crate) fn retain_result(&self, value: serde_json::Value) -> Result<(), String> {
        self.inner.lock().map_err(|_| "durable session lock poisoned")?.result_json = Some(value);
        Ok(())
    }

    /// Preserve the JSON number representation validated by the JavaScript engine.
    pub fn take_result(&self) -> Option<serde_json::Value> {
        self.inner.lock().ok()?.result_json.take()
    }

    /// Extract only the authority needed for terminal persistence. No replay
    /// history or engine state is retained by this handle.
    pub fn completion_handle(&self) -> Result<DurableCompletion, DurableError> {
        let inner = self.inner.lock().map_err(|_| DurableError::LockPoisoned)?;
        if inner.storage_error.is_some()
            || inner.replay.ensure_consumed().is_err()
            || inner.active_wakeup.is_some()
            || inner.suspended
        {
            return Err(DurableError::TaskSuspended { task_id: inner.task_id });
        }
        Ok(DurableCompletion {
            store: inner.store.clone(),
            task_id: inner.task_id,
            next_sequence: inner.next_sequence,
            lease_until_ms: inner.lease_until_ms,
        })
    }

    /// Persist successful termination after complete replay. Persistent suspension
    /// checks run atomically inside the store's completion transaction.
    pub fn complete(&self, value: &serde_json::Value) -> Result<bool, DurableError> {
        self.completion_handle()?.complete(value)
    }

    pub(crate) fn is_suspended(&self) -> Result<bool, String> {
        let inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        Ok(inner.suspended)
    }

    pub(crate) fn ensure_consumed(&self) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.check_storage()?;
        inner.replay.ensure_consumed().map_err(replay_error)?;
        // Storage checks belong to terminal persistence, where availability
        // errors can be retried with the computed result. Keep this check local.
        if inner.active_wakeup.is_some() || inner.suspended {
            return Err("durable execution returned with a persisted suspension".into());
        }
        Ok(())
    }
}

/// Lightweight terminal write authority; deliberately contains no replay cursor.
pub struct DurableCompletion {
    store: Arc<dyn DurableStore>,
    task_id: TaskId,
    next_sequence: u64,
    lease_until_ms: Option<u64>,
}

impl DurableCompletion {
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    pub fn complete(&self, value: &serde_json::Value) -> Result<bool, DurableError> {
        if let Some(until) = self.lease_until_ms {
            let result =
                self.store.complete_task_before(self.task_id, self.next_sequence, value, until);
            // A deadline can expire inside the completion transaction after its
            // initial fencing check. Normalize that case to lost authority too.
            if matches!(&result, Err(DurableError::TaskSuspended { .. }))
                && std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .is_ok_and(|now| now.as_millis() >= u128::from(until))
            {
                return Err(DurableError::ExecutionLeaseLost);
            }
            return result;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_millis() as u64;
        self.store.complete_task(self.task_id, self.next_sequence, value, now_ms)
    }
}

fn parse_kind(raw: &str) -> Result<EventKind, String> {
    match raw {
        "step" => Ok(EventKind::Step),
        "effect" => Ok(EventKind::Effect),
        "sleep" => Ok(EventKind::Sleep),
        "signal" => Ok(EventKind::Signal),
        "retry" => Ok(EventKind::Retry),
        "now" => Ok(EventKind::Now),
        "random" => Ok(EventKind::Random),
        _ => Err(format!("unknown durable event kind {raw:?}")),
    }
}

fn replay_error(error: ReplayError) -> String {
    error.to_string()
}

fn unix_time_ms() -> Result<u64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?;
    u64::try_from(duration.as_millis()).map_err(|_| "system time is too large".into())
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    use tysel_durable::SqliteStore;

    #[test]
    fn completion_handle_does_not_retain_consumed_history() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(982);
        store.put_program(id, "42", 0).unwrap();
        store
            .append_event_json_at(
                id,
                0,
                EventKind::Step,
                "large".into(),
                &serde_json::to_string(&"x".repeat(1_000_000)).unwrap(),
                0,
            )
            .unwrap();
        let session = DurableSession::new(store.clone(), id).unwrap();
        session.lookup_json("step", "large").unwrap();
        let weak = Arc::downgrade(&session.inner);
        let completion = session.completion_handle().unwrap();
        drop(session);
        assert!(weak.upgrade().is_none(), "finalization must release the replay owner");
        assert!(completion.complete(&json!(42)).unwrap());
    }

    #[test]
    fn completion_transaction_rejects_a_new_persisted_wait() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(983);
        store.put_program(id, "42", 0).unwrap();
        let session = DurableSession::new(store.clone(), id).unwrap();
        let completion = session.completion_handle().unwrap();
        store.poll_signal(id, 0, "late", 0, None).unwrap();
        assert!(matches!(completion.complete(&json!(42)), Err(DurableError::TaskSuspended { .. })));
        assert!(store.completion(id).unwrap().is_none());
    }

    #[test]
    fn expired_completion_can_only_acknowledge_an_identical_committed_outcome() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let task_id = TaskId(981);
        store.put_program(task_id, "42", 0).unwrap();
        let session = DurableSession::new(store.clone(), task_id).unwrap();
        session.inner.lock().unwrap().lease_until_ms = Some(0);
        assert!(matches!(session.complete(&json!(42)), Err(DurableError::ExecutionLeaseLost)));
        assert!(store.completion(task_id).unwrap().is_none());
        store.complete_task(task_id, 0, &json!(42), 1).unwrap();
        assert!(session.complete(&json!(42)).unwrap());
        assert!(matches!(session.complete(&json!(43)), Err(DurableError::TaskCompleted { .. })));
        assert_eq!(store.completion(task_id).unwrap().unwrap().value, json!(42));
    }
}
