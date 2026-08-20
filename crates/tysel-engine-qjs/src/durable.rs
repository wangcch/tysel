use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tysel_durable::{EventKind, NewEvent, ReplayCursor, ReplayError, SqliteStore, WakeupClaim};
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
}

struct WakeupToken {
    sequence: u64,
    lease_owner: Option<String>,
}

impl DurableSession {
    /// Start a task that has no pending wakeup. Suspended tasks must enter via
    /// `from_claim` so they cannot resume early or run under two schedulers.
    pub fn new(store: Arc<SqliteStore>, task_id: TaskId) -> Result<Self, String> {
        if let Some(wakeup) = store.wakeup(task_id).map_err(|err| err.to_string())? {
            return Err(format!(
                "durable task is suspended until {} and must be resumed from a wakeup claim",
                wakeup.wake_at_ms
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
        Self::load(
            store,
            task_id,
            Some(WakeupToken { sequence: claim.sequence, lease_owner: Some(claim.lease_owner) }),
        )
    }

    fn load(
        store: Arc<SqliteStore>,
        task_id: TaskId,
        active_wakeup: Option<WakeupToken>,
    ) -> Result<Self, String> {
        let history = store.load_history(task_id).map_err(|err| err.to_string())?;
        if let Some(token) = &active_wakeup
            && !history
                .events
                .iter()
                .any(|event| event.sequence == token.sequence && event.kind == EventKind::Sleep)
        {
            return Err("durable wakeup does not reference a recorded sleep event".into());
        }
        let next_sequence = history
            .events
            .last()
            .map(|event| event.sequence.checked_add(1).ok_or("durable history is too large"))
            .transpose()?
            .unwrap_or(0);
        let replay = history.replay();
        Ok(Self {
            inner: Arc::new(Mutex::new(SessionInner {
                store,
                task_id,
                replay,
                next_sequence,
                active_wakeup,
            })),
        })
    }

    pub(crate) fn lookup_json(&self, kind: &str, key: &str) -> Result<String, String> {
        let kind = parse_kind(kind)?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let event = inner.replay.consume_event(kind, key).map_err(replay_error)?.cloned();
        serde_json::to_string(&match event {
            Some(event) => json!({
                "found": true,
                "payload": event.payload,
                "sequence": event.sequence,
                "recordedAtMs": event.recorded_at_ms,
            }),
            None => json!({ "found": false }),
        })
        .map_err(|err| err.to_string())
    }

    pub(crate) fn record(
        &self,
        kind: &str,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
    ) -> Result<(), String> {
        let kind = parse_kind(kind)?;
        let payload: Value = serde_json::from_str(payload_json).map_err(|err| err.to_string())?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let stored = inner
            .store
            .append_event_at(
                inner.task_id,
                inner.next_sequence,
                NewEvent { kind, key, payload, recorded_at_ms },
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
        let payload: Value = serde_json::from_str(payload_json).map_err(|err| err.to_string())?;
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let stored = inner
            .store
            .append_event_with_wakeup_at(
                inner.task_id,
                inner.next_sequence,
                NewEvent { kind: EventKind::Sleep, key, payload, recorded_at_ms },
                wake_at_ms,
            )
            .map_err(|err| err.to_string())?;
        inner.next_sequence = stored.sequence.saturating_add(1);
        inner.active_wakeup = Some(WakeupToken { sequence: stored.sequence, lease_owner: None });
        Ok(())
    }

    pub(crate) fn complete_sleep(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        let token = inner
            .active_wakeup
            .as_ref()
            .ok_or_else(|| "durable sleep has no active wakeup claim".to_string())?;
        let completed = inner
            .store
            .complete_wakeup(
                inner.task_id,
                token.sequence,
                token.lease_owner.as_deref(),
                unix_time_ms()?,
            )
            .map_err(|err| err.to_string())?;
        if !completed {
            return Err("durable wakeup ownership was lost".into());
        }
        inner.active_wakeup = None;
        Ok(())
    }

    pub(crate) fn ensure_consumed(&self) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|_| "durable session lock poisoned")?;
        inner.replay.ensure_consumed().map_err(replay_error)
    }
}

fn parse_kind(raw: &str) -> Result<EventKind, String> {
    match raw {
        "step" => Ok(EventKind::Step),
        "effect" => Ok(EventKind::Effect),
        "sleep" => Ok(EventKind::Sleep),
        "signal" => Ok(EventKind::Signal),
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
