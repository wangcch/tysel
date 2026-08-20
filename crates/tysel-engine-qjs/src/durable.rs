use std::sync::{Arc, Mutex};

use serde_json::json;
use tysel_durable::{EventKind, ReplayCursor, ReplayError, SqliteStore, WakeupClaim};
use tysel_task::TaskId;

#[derive(Clone)]
pub struct DurableSession {
    inner: Arc<Mutex<SessionInner>>,
}

struct SessionInner {
    store: Arc<SqliteStore>,
    task_id: TaskId,
    replay: ReplayCursor,
    next_sequence: u64,
    active_wakeup: Option<WakeupToken>,
    suspended: bool,
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
            .map_err(|err| err.to_string())?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        Ok(stored.payload_json().into())
    }

    /// Start a task that has no pending wakeup. Suspended tasks must enter via
    /// `from_claim` so they cannot resume early or run under two schedulers.
    pub fn new(store: Arc<SqliteStore>, task_id: TaskId) -> Result<Self, String> {
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

    pub fn from_claim(store: Arc<SqliteStore>, claim: WakeupClaim) -> Result<Self, String> {
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

    fn load(
        store: Arc<SqliteStore>,
        task_id: TaskId,
        active_claim: Option<WakeupClaim>,
    ) -> Result<Self, String> {
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
            })),
        })
    }

    pub(crate) fn lookup_json(&self, kind: &str, key: &str) -> Result<String, String> {
        let kind = parse_kind(kind)?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let event = inner.replay.consume_event(kind, key).map_err(replay_error)?.cloned();
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
            .map_err(|err| err.to_string())?;
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
            .map_err(|err| err.to_string())?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        inner.active_wakeup =
            Some(WakeupToken { sequence: stored.sequence, kind: WakeupKind::Sleep, claim: None });
        inner.suspended = true;
        Ok(())
    }

    pub(crate) fn complete_sleep(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
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
            .map_err(|err| err.to_string())?;
        if !completed {
            return Err("durable wakeup ownership was lost".into());
        }
        inner.active_wakeup = None;
        Ok(())
    }

    pub(crate) fn poll_signal_json(&self, signal_name: &str) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let claim = inner
            .active_wakeup
            .as_ref()
            .filter(|token| token.kind == WakeupKind::Signal)
            .and_then(|token| token.claim.as_ref());
        let event = inner
            .store
            .poll_signal(inner.task_id, inner.next_sequence, signal_name, unix_time_ms()?, claim)
            .map_err(|err| err.to_string())?;
        let response = if let Some(event) = event {
            inner.next_sequence = event.sequence.saturating_add(1);
            if inner.active_wakeup.as_ref().is_some_and(|token| {
                token.kind == WakeupKind::Signal && token.sequence == event.sequence
            }) {
                inner.active_wakeup = None;
            }
            json!({ "found": true, "payload": event.payload })
        } else {
            inner.suspended = true;
            json!({ "found": false })
        };
        serde_json::to_string(&response).map_err(|err| err.to_string())
    }

    pub(crate) fn is_suspended(&self) -> Result<bool, String> {
        let inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        Ok(inner.suspended)
    }

    pub(crate) fn ensure_consumed(&self) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.replay.ensure_consumed().map_err(replay_error)?;
        let persisted_suspension =
            inner.store.wakeup(inner.task_id).map_err(|err| err.to_string())?.is_some()
                || inner.store.signal_wait(inner.task_id).map_err(|err| err.to_string())?.is_some();
        if inner.active_wakeup.is_some() || persisted_suspension {
            return Err("durable execution returned with a persisted suspension".into());
        }
        Ok(())
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
