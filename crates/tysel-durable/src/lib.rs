//! Durable task event history, deterministic replay, and SQLite wakeup storage.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use tysel_task::TaskId;

const MAX_WAKEUP_BATCH: usize = 10_000;
const MAX_EVENT_KEY_BYTES: usize = 256;
const MAX_EVENT_PAYLOAD_BYTES: usize = 1_048_576;
const MAX_HISTORY_EVENTS: usize = 10_000;
const MAX_HISTORY_BYTES: usize = 16 * 1_048_576;
const MAX_LEASE_OWNER_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Step,
    Effect,
    Sleep,
    Signal,
    Now,
    Random,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Effect => "effect",
            Self::Sleep => "sleep",
            Self::Signal => "signal",
            Self::Now => "now",
            Self::Random => "random",
        }
    }

    fn parse(raw: &str) -> Result<Self, DurableError> {
        match raw {
            "step" => Ok(Self::Step),
            "effect" => Ok(Self::Effect),
            "sleep" => Ok(Self::Sleep),
            "signal" => Ok(Self::Signal),
            "now" => Ok(Self::Now),
            "random" => Ok(Self::Random),
            _ => Err(DurableError::InvalidEventKind(raw.into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub kind: EventKind,
    pub key: String,
    pub payload: Value,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskEvent {
    pub task_id: TaskId,
    pub sequence: u64,
    pub kind: EventKind,
    pub key: String,
    pub payload: Value,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    pub task_id: TaskId,
    pub events: Vec<TaskEvent>,
}

impl History {
    pub fn replay(self) -> ReplayCursor {
        ReplayCursor { events: self.events, position: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayCursor {
    events: Vec<TaskEvent>,
    position: usize,
}

impl ReplayCursor {
    /// Consume the next recorded durable boundary. `Ok(None)` means execution
    /// has reached a new boundary that is not present in history yet.
    pub fn consume(&mut self, kind: EventKind, key: &str) -> Result<Option<&Value>, ReplayError> {
        Ok(self.consume_event(kind, key)?.map(|event| &event.payload))
    }

    pub fn consume_event(
        &mut self,
        kind: EventKind,
        key: &str,
    ) -> Result<Option<&TaskEvent>, ReplayError> {
        let Some(event) = self.events.get(self.position) else {
            return Ok(None);
        };
        if event.kind != kind || event.key != key {
            return Err(ReplayError::Mismatch {
                sequence: event.sequence,
                expected_kind: event.kind,
                expected_key: event.key.clone(),
                actual_kind: kind,
                actual_key: key.into(),
            });
        }
        self.position += 1;
        Ok(Some(event))
    }

    pub fn ensure_consumed(&self) -> Result<(), ReplayError> {
        if let Some(event) = self.events.get(self.position) {
            return Err(ReplayError::HistoryRemaining {
                sequence: event.sequence,
                kind: event.kind,
                key: event.key.clone(),
            });
        }
        Ok(())
    }

    pub fn position(&self) -> usize {
        self.position
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    #[error(
        "replay mismatch at sequence {sequence}: expected {expected_kind:?}/{expected_key:?}, got {actual_kind:?}/{actual_key:?}"
    )]
    Mismatch {
        sequence: u64,
        expected_kind: EventKind,
        expected_key: String,
        actual_kind: EventKind,
        actual_key: String,
    },
    #[error("replay stopped before sequence {sequence} ({kind:?}/{key:?})")]
    HistoryRemaining { sequence: u64, kind: EventKind, key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wakeup {
    pub task_id: TaskId,
    pub sequence: u64,
    pub wake_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeupClaim {
    pub task_id: TaskId,
    pub sequence: u64,
    pub wake_at_ms: u64,
    pub lease_owner: String,
    pub lease_until_ms: u64,
}

pub struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DurableError> {
        if let Some(parent) = path.as_ref().parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, DurableError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, DurableError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS durable_events (
                 task_id BLOB NOT NULL,
                 sequence INTEGER NOT NULL CHECK (sequence >= 0),
                 kind TEXT NOT NULL,
                 event_key TEXT NOT NULL,
                 payload TEXT NOT NULL,
                 recorded_at_ms INTEGER NOT NULL CHECK (recorded_at_ms >= 0),
                 PRIMARY KEY (task_id, sequence)
             );
             CREATE TABLE IF NOT EXISTS durable_wakeups (
                 task_id BLOB PRIMARY KEY,
                 sequence INTEGER NOT NULL CHECK (sequence >= 0),
                 wake_at_ms INTEGER NOT NULL CHECK (wake_at_ms >= 0),
                 lease_owner TEXT,
                 lease_until_ms INTEGER CHECK (lease_until_ms >= 0)
             );
             CREATE TABLE IF NOT EXISTS durable_history_stats (
                 task_id BLOB PRIMARY KEY,
                 event_count INTEGER NOT NULL CHECK (event_count >= 0),
                 payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 0)
             );",
        )?;
        migrate_wakeup_columns(&connection)?;
        connection.execute_batch(
            "DROP INDEX IF EXISTS durable_wakeups_due;
             CREATE INDEX durable_wakeups_due
                 ON durable_wakeups (wake_at_ms, lease_until_ms, task_id);
             UPDATE durable_wakeups
             SET sequence = COALESCE((
                 SELECT MAX(sequence) FROM durable_events
                 WHERE durable_events.task_id = durable_wakeups.task_id
                   AND kind = 'sleep'
             ), sequence);
             INSERT OR REPLACE INTO durable_history_stats
                 (task_id, event_count, payload_bytes)
             SELECT task_id, COUNT(*), COALESCE(SUM(
                 length(CAST(payload AS BLOB)) + length(CAST(event_key AS BLOB))
             ), 0)
             FROM durable_events GROUP BY task_id;",
        )?;
        Ok(Self { connection: Mutex::new(connection) })
    }

    /// Append under an IMMEDIATE transaction so concurrent writers allocate a
    /// unique, gap-free sequence for each task history.
    pub fn append_event(
        &self,
        task_id: TaskId,
        event: NewEvent,
    ) -> Result<TaskEvent, DurableError> {
        self.append_event_inner(task_id, None, event)
    }

    /// Append only if the task history still has `expected_sequence` as its
    /// next position. Durable executors use this to reject stale concurrent runs.
    pub fn append_event_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        event: NewEvent,
    ) -> Result<TaskEvent, DurableError> {
        self.append_event_inner(task_id, Some(expected_sequence), event)
    }

    fn append_event_inner(
        &self,
        task_id: TaskId,
        expected_sequence: Option<u64>,
        event: NewEvent,
    ) -> Result<TaskEvent, DurableError> {
        let (payload, recorded_at_ms) = validate_event(&event)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = insert_event(
            &transaction,
            task_id,
            expected_sequence,
            &event,
            &payload,
            recorded_at_ms,
        )?;
        transaction.commit()?;
        stored_event(task_id, sequence, event)
    }

    /// Record a sleep boundary and its wakeup atomically. A crash can expose
    /// either neither write or both writes, but never a suspended task without
    /// a timer.
    pub fn append_event_with_wakeup(
        &self,
        task_id: TaskId,
        event: NewEvent,
        wake_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        self.append_event_with_wakeup_inner(task_id, None, event, wake_at_ms)
    }

    pub fn append_event_with_wakeup_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        event: NewEvent,
        wake_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        self.append_event_with_wakeup_inner(task_id, Some(expected_sequence), event, wake_at_ms)
    }

    fn append_event_with_wakeup_inner(
        &self,
        task_id: TaskId,
        expected_sequence: Option<u64>,
        event: NewEvent,
        wake_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        let (payload, recorded_at_ms) = validate_event(&event)?;
        let wake_at_ms_sql = to_sql_integer(wake_at_ms, "wake_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = insert_event(
            &transaction,
            task_id,
            expected_sequence,
            &event,
            &payload,
            recorded_at_ms,
        )?;
        upsert_wakeup(&transaction, task_id, sequence, wake_at_ms_sql)?;
        transaction.commit()?;
        stored_event(task_id, sequence, event)
    }

    pub fn load_history(&self, task_id: TaskId) -> Result<History, DurableError> {
        let connection = self.lock()?;
        let id = task_id_bytes(task_id);
        let mut statement = connection.prepare(
            "SELECT sequence, kind, event_key, payload, recorded_at_ms
             FROM durable_events WHERE task_id = ?1 ORDER BY sequence LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![id.as_slice(), (MAX_HISTORY_EVENTS + 1) as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        let mut history_bytes = 0usize;
        for row in rows {
            let (sequence, kind, key, payload, recorded_at_ms) = row?;
            if events.len() >= MAX_HISTORY_EVENTS {
                return Err(DurableError::HistoryEventLimit);
            }
            history_bytes = history_bytes
                .checked_add(key.len())
                .and_then(|bytes| bytes.checked_add(payload.len()))
                .ok_or(DurableError::HistoryByteLimit)?;
            if history_bytes > MAX_HISTORY_BYTES {
                return Err(DurableError::HistoryByteLimit);
            }
            events.push(TaskEvent {
                task_id,
                sequence: from_sql_integer(sequence, "sequence")?,
                kind: EventKind::parse(&kind)?,
                key,
                payload: serde_json::from_str(&payload)?,
                recorded_at_ms: from_sql_integer(recorded_at_ms, "recorded_at_ms")?,
            });
        }
        Ok(History { task_id, events })
    }

    pub fn schedule_wakeup(&self, wakeup: Wakeup) -> Result<(), DurableError> {
        let connection = self.lock()?;
        upsert_wakeup(
            &connection,
            wakeup.task_id,
            to_sql_integer(wakeup.sequence, "sequence")?,
            to_sql_integer(wakeup.wake_at_ms, "wake_at_ms")?,
        )?;
        Ok(())
    }

    /// Read due wakeups for diagnostics only. Executors must atomically claim
    /// them with `claim_due_wakeups` before starting work.
    pub fn peek_due_wakeups(&self, now_ms: u64, limit: usize) -> Result<Vec<Wakeup>, DurableError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.lock()?;
        let limit = limit.min(MAX_WAKEUP_BATCH);
        let mut statement = connection.prepare(
            "SELECT task_id, sequence, wake_at_ms FROM durable_wakeups
             WHERE wake_at_ms <= ?1 ORDER BY wake_at_ms, task_id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![to_sql_integer(now_ms, "now_ms")?, limit as i64], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            })?;
        let mut wakeups = Vec::new();
        for row in rows {
            let (task_id, sequence, wake_at_ms) = row?;
            wakeups.push(Wakeup {
                task_id: task_id_from_bytes(&task_id)?,
                sequence: from_sql_integer(sequence, "sequence")?,
                wake_at_ms: from_sql_integer(wake_at_ms, "wake_at_ms")?,
            });
        }
        Ok(wakeups)
    }

    /// Lease due wakeups in one transaction. An expired lease is claimable
    /// again after a runner crash; an active lease excludes other runners.
    pub fn claim_due_wakeups(
        &self,
        now_ms: u64,
        limit: usize,
        lease_owner: &str,
        lease_duration_ms: u64,
    ) -> Result<Vec<WakeupClaim>, DurableError> {
        validate_lease_owner(lease_owner)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let lease_until_sql = to_sql_integer(lease_until_ms, "lease_until_ms")?;
        let limit = limit.min(MAX_WAKEUP_BATCH);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates = {
            let mut statement = transaction.prepare(
                "SELECT task_id, sequence, wake_at_ms FROM durable_wakeups
                 WHERE wake_at_ms <= ?1
                   AND (lease_until_ms IS NULL OR lease_until_ms <= ?1)
                 ORDER BY wake_at_ms, task_id LIMIT ?2",
            )?;
            let rows = statement.query_map(params![now_sql, limit as i64], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut claims = Vec::with_capacity(candidates.len());
        for (id, sequence, wake_at_ms) in candidates {
            let changed = transaction.execute(
                "UPDATE durable_wakeups
                 SET lease_owner = ?1, lease_until_ms = ?2
                 WHERE task_id = ?3 AND sequence = ?4
                   AND wake_at_ms <= ?5
                   AND (lease_until_ms IS NULL OR lease_until_ms <= ?5)",
                params![lease_owner, lease_until_sql, id.as_slice(), sequence, now_sql],
            )?;
            if changed == 1 {
                claims.push(WakeupClaim {
                    task_id: task_id_from_bytes(&id)?,
                    sequence: from_sql_integer(sequence, "sequence")?,
                    wake_at_ms: from_sql_integer(wake_at_ms, "wake_at_ms")?,
                    lease_owner: lease_owner.into(),
                    lease_until_ms,
                });
            }
        }
        transaction.commit()?;
        Ok(claims)
    }

    /// Complete only the exact wakeup generation owned by this execution.
    pub fn complete_wakeup(
        &self,
        task_id: TaskId,
        sequence: u64,
        lease_owner: Option<&str>,
        now_ms: u64,
    ) -> Result<bool, DurableError> {
        let connection = self.lock()?;
        let id = task_id_bytes(task_id);
        let sequence = to_sql_integer(sequence, "sequence")?;
        let now_ms = to_sql_integer(now_ms, "now_ms")?;
        let changed = match lease_owner {
            Some(owner) => {
                validate_lease_owner(owner)?;
                connection.execute(
                    "DELETE FROM durable_wakeups
                     WHERE task_id = ?1 AND sequence = ?2 AND lease_owner = ?3
                       AND lease_until_ms > ?4",
                    params![id.as_slice(), sequence, owner, now_ms],
                )?
            }
            None => connection.execute(
                "DELETE FROM durable_wakeups
                 WHERE task_id = ?1 AND sequence = ?2 AND lease_owner IS NULL",
                params![id.as_slice(), sequence],
            )?,
        };
        Ok(changed == 1)
    }

    pub fn claim_is_active(&self, claim: &WakeupClaim, now_ms: u64) -> Result<bool, DurableError> {
        let connection = self.lock()?;
        let id = task_id_bytes(claim.task_id);
        let found = connection
            .query_row(
                "SELECT 1 FROM durable_wakeups
                 WHERE task_id = ?1 AND sequence = ?2 AND wake_at_ms = ?3
                   AND lease_owner = ?4 AND lease_until_ms = ?5
                   AND lease_until_ms > ?6",
                params![
                    id.as_slice(),
                    to_sql_integer(claim.sequence, "sequence")?,
                    to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                    claim.lease_owner,
                    to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                    to_sql_integer(now_ms, "now_ms")?
                ],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn wakeup(&self, task_id: TaskId) -> Result<Option<Wakeup>, DurableError> {
        let connection = self.lock()?;
        let id = task_id_bytes(task_id);
        let wakeup = connection
            .query_row(
                "SELECT sequence, wake_at_ms FROM durable_wakeups WHERE task_id = ?1",
                params![id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        wakeup
            .map(|(sequence, wake_at_ms)| {
                Ok(Wakeup {
                    task_id,
                    sequence: from_sql_integer(sequence, "sequence")?,
                    wake_at_ms: from_sql_integer(wake_at_ms, "wake_at_ms")?,
                })
            })
            .transpose()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DurableError> {
        self.connection.lock().map_err(|_| DurableError::LockPoisoned)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DurableError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("durable store lock is poisoned")]
    LockPoisoned,
    #[error("unknown durable event kind {0:?}")]
    InvalidEventKind(String),
    #[error("invalid durable task id blob")]
    InvalidTaskId,
    #[error("{field} is outside SQLite's signed integer range")]
    IntegerRange { field: &'static str },
    #[error("durable event key exceeds {MAX_EVENT_KEY_BYTES} bytes")]
    EventKeyTooLarge,
    #[error("durable event payload exceeds {MAX_EVENT_PAYLOAD_BYTES} bytes")]
    EventPayloadTooLarge,
    #[error("durable history conflict: expected next sequence {expected}, found {actual}")]
    HistoryConflict { expected: u64, actual: u64 },
    #[error("durable history exceeds {MAX_HISTORY_EVENTS} events")]
    HistoryEventLimit,
    #[error("durable history exceeds {MAX_HISTORY_BYTES} bytes")]
    HistoryByteLimit,
    #[error("lease owner must be 1..={MAX_LEASE_OWNER_BYTES} bytes")]
    InvalidLeaseOwner,
}

fn validate_event(event: &NewEvent) -> Result<(String, i64), DurableError> {
    if event.key.len() > MAX_EVENT_KEY_BYTES {
        return Err(DurableError::EventKeyTooLarge);
    }
    let payload = serde_json::to_string(&event.payload)?;
    if payload.len() > MAX_EVENT_PAYLOAD_BYTES {
        return Err(DurableError::EventPayloadTooLarge);
    }
    Ok((payload, to_sql_integer(event.recorded_at_ms, "recorded_at_ms")?))
}

fn insert_event(
    transaction: &Transaction<'_>,
    task_id: TaskId,
    expected_sequence: Option<u64>,
    event: &NewEvent,
    payload: &str,
    recorded_at_ms: i64,
) -> Result<i64, DurableError> {
    let id = task_id_bytes(task_id);
    let event_bytes =
        event.key.len().checked_add(payload.len()).ok_or(DurableError::HistoryByteLimit)?;
    let stats = transaction
        .query_row(
            "SELECT event_count, payload_bytes FROM durable_history_stats WHERE task_id = ?1",
            params![id.as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let (event_count, payload_bytes) = stats.unwrap_or((0, 0));
    if event_count >= MAX_HISTORY_EVENTS as i64 {
        return Err(DurableError::HistoryEventLimit);
    }
    let next_payload_bytes = payload_bytes
        .checked_add(i64::try_from(event_bytes).map_err(|_| DurableError::HistoryByteLimit)?)
        .ok_or(DurableError::HistoryByteLimit)?;
    if next_payload_bytes > MAX_HISTORY_BYTES as i64 {
        return Err(DurableError::HistoryByteLimit);
    }
    let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence) + 1, 0)
         FROM durable_events WHERE task_id = ?1",
        params![id.as_slice()],
        |row| row.get(0),
    )?;
    if let Some(expected) = expected_sequence {
        let actual = from_sql_integer(sequence, "sequence")?;
        if actual != expected {
            return Err(DurableError::HistoryConflict { expected, actual });
        }
    }
    transaction.execute(
        "INSERT INTO durable_events
         (task_id, sequence, kind, event_key, payload, recorded_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.as_slice(), sequence, event.kind.as_str(), event.key, payload, recorded_at_ms],
    )?;
    transaction.execute(
        "INSERT INTO durable_history_stats (task_id, event_count, payload_bytes)
         VALUES (?1, 1, ?2)
         ON CONFLICT(task_id) DO UPDATE SET
             event_count = durable_history_stats.event_count + 1,
             payload_bytes = durable_history_stats.payload_bytes + excluded.payload_bytes",
        params![id.as_slice(), event_bytes as i64],
    )?;
    Ok(sequence)
}

fn upsert_wakeup(
    connection: &Connection,
    task_id: TaskId,
    sequence: i64,
    wake_at_ms: i64,
) -> Result<(), DurableError> {
    let id = task_id_bytes(task_id);
    connection.execute(
        "INSERT INTO durable_wakeups
             (task_id, sequence, wake_at_ms, lease_owner, lease_until_ms)
         VALUES (?1, ?2, ?3, NULL, NULL)
         ON CONFLICT(task_id) DO UPDATE SET
             sequence = excluded.sequence,
             wake_at_ms = excluded.wake_at_ms,
             lease_owner = NULL,
             lease_until_ms = NULL",
        params![id.as_slice(), sequence, wake_at_ms],
    )?;
    Ok(())
}

fn validate_lease_owner(owner: &str) -> Result<(), DurableError> {
    if owner.is_empty() || owner.len() > MAX_LEASE_OWNER_BYTES {
        return Err(DurableError::InvalidLeaseOwner);
    }
    Ok(())
}

fn migrate_wakeup_columns(connection: &Connection) -> Result<(), DurableError> {
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(durable_wakeups)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if !columns.iter().any(|column| column == "sequence") {
        connection.execute_batch(
            "ALTER TABLE durable_wakeups ADD COLUMN sequence INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    if !columns.iter().any(|column| column == "lease_owner") {
        connection.execute_batch("ALTER TABLE durable_wakeups ADD COLUMN lease_owner TEXT")?;
    }
    if !columns.iter().any(|column| column == "lease_until_ms") {
        connection
            .execute_batch("ALTER TABLE durable_wakeups ADD COLUMN lease_until_ms INTEGER")?;
    }
    Ok(())
}

fn stored_event(
    task_id: TaskId,
    sequence: i64,
    event: NewEvent,
) -> Result<TaskEvent, DurableError> {
    Ok(TaskEvent {
        task_id,
        sequence: from_sql_integer(sequence, "sequence")?,
        kind: event.kind,
        key: event.key,
        payload: event.payload,
        recorded_at_ms: event.recorded_at_ms,
    })
}

fn task_id_bytes(task_id: TaskId) -> [u8; 16] {
    task_id.0.to_be_bytes()
}

fn task_id_from_bytes(bytes: &[u8]) -> Result<TaskId, DurableError> {
    let bytes: [u8; 16] = bytes.try_into().map_err(|_| DurableError::InvalidTaskId)?;
    Ok(TaskId(u128::from_be_bytes(bytes)))
}

fn to_sql_integer(value: u64, field: &'static str) -> Result<i64, DurableError> {
    i64::try_from(value).map_err(|_| DurableError::IntegerRange { field })
}

fn from_sql_integer(value: i64, field: &'static str) -> Result<u64, DurableError> {
    u64::try_from(value).map_err(|_| DurableError::IntegerRange { field })
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use serde_json::json;

    use super::*;

    fn event(kind: EventKind, key: &str, value: Value, at: u64) -> NewEvent {
        NewEvent { kind, key: key.into(), payload: value, recorded_at_ms: at }
    }

    #[test]
    fn history_is_ordered_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = TaskId(7);
        {
            let store = SqliteStore::open(&path).unwrap();
            let first = store
                .append_event(id, event(EventKind::Step, "load", json!({"n": 1}), 10))
                .unwrap();
            let second =
                store.append_event(id, event(EventKind::Effect, "send", json!("ok"), 11)).unwrap();
            assert_eq!((first.sequence, second.sequence), (0, 1));
        }
        let history = SqliteStore::open(&path).unwrap().load_history(id).unwrap();
        assert_eq!(history.events.len(), 2);
        assert_eq!(history.events[0].key, "load");
        assert_eq!(history.events[1].payload, json!("ok"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn existing_wakeup_schema_is_migrated_without_losing_timer() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-migrate-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = task_id_bytes(TaskId(12));
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE durable_events (
                         task_id BLOB NOT NULL, sequence INTEGER NOT NULL, kind TEXT NOT NULL,
                         event_key TEXT NOT NULL, payload TEXT NOT NULL, recorded_at_ms INTEGER NOT NULL,
                         PRIMARY KEY (task_id, sequence)
                     );
                     CREATE TABLE durable_wakeups (
                         task_id BLOB PRIMARY KEY, wake_at_ms INTEGER NOT NULL
                     );",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO durable_events VALUES (?1, 3, 'sleep', 'sleep:5', '{}', 10)",
                    params![id.as_slice()],
                )
                .unwrap();
            connection
                .execute("INSERT INTO durable_wakeups VALUES (?1, 15)", params![id.as_slice()])
                .unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(
            store.wakeup(TaskId(12)).unwrap(),
            Some(Wakeup { task_id: TaskId(12), sequence: 3, wake_at_ms: 15 })
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_appends_allocate_unique_sequences() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = TaskId(9);
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    store
                        .append_event(
                            id,
                            event(EventKind::Step, &format!("step-{index}"), json!(index), index),
                        )
                        .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let history = store.load_history(id).unwrap();
        let sequences: Vec<_> = history.events.iter().map(|event| event.sequence).collect();
        assert_eq!(sequences, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn expected_sequence_rejects_a_stale_writer() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(10);
        store.append_event_at(id, 0, event(EventKind::Step, "first", json!(1), 1)).unwrap();
        let err =
            store.append_event_at(id, 0, event(EventKind::Step, "stale", json!(2), 2)).unwrap_err();
        assert!(matches!(err, DurableError::HistoryConflict { expected: 0, actual: 1 }));
        assert_eq!(store.load_history(id).unwrap().events.len(), 1);
    }

    #[test]
    fn replay_returns_recorded_values_and_detects_divergence() {
        let history = History {
            task_id: TaskId(1),
            events: vec![
                TaskEvent {
                    task_id: TaskId(1),
                    sequence: 0,
                    kind: EventKind::Now,
                    key: "clock".into(),
                    payload: json!(1234),
                    recorded_at_ms: 1234,
                },
                TaskEvent {
                    task_id: TaskId(1),
                    sequence: 1,
                    kind: EventKind::Step,
                    key: "next".into(),
                    payload: json!({"ok": true}),
                    recorded_at_ms: 1235,
                },
            ],
        };
        let mut replay = history.replay();
        assert_eq!(replay.consume(EventKind::Now, "clock").unwrap(), Some(&json!(1234)));
        let err = replay.consume(EventKind::Effect, "next").unwrap_err();
        assert!(matches!(err, ReplayError::Mismatch { sequence: 1, .. }));
        assert_eq!(replay.position(), 1);
    }

    #[test]
    fn replay_reports_unconsumed_history_and_then_new_boundary() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(3);
        store.append_event(id, event(EventKind::Step, "one", json!(1), 1)).unwrap();
        let mut replay = store.load_history(id).unwrap().replay();
        assert!(matches!(replay.ensure_consumed(), Err(ReplayError::HistoryRemaining { .. })));
        replay.consume(EventKind::Step, "one").unwrap();
        replay.ensure_consumed().unwrap();
        assert_eq!(replay.consume(EventKind::Step, "two").unwrap(), None);
    }

    #[test]
    fn wakeup_claims_are_exclusive_and_expire() {
        let store = SqliteStore::in_memory().unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(1), sequence: 1, wake_at_ms: 30 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(2), sequence: 4, wake_at_ms: 10 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(1), sequence: 2, wake_at_ms: 20 }).unwrap();
        assert_eq!(
            store.peek_due_wakeups(20, 10).unwrap(),
            vec![
                Wakeup { task_id: TaskId(2), sequence: 4, wake_at_ms: 10 },
                Wakeup { task_id: TaskId(1), sequence: 2, wake_at_ms: 20 }
            ]
        );
        let claims = store.claim_due_wakeups(20, 10, "runner-a", 100).unwrap();
        assert_eq!(claims.len(), 2);
        assert!(store.claim_due_wakeups(20, 10, "runner-b", 100).unwrap().is_empty());
        assert!(store.claim_due_wakeups(119, 10, "runner-b", 100).unwrap().is_empty());
        assert!(!store.complete_wakeup(TaskId(1), 2, Some("runner-a"), 120).unwrap());
        let reclaimed = store.claim_due_wakeups(120, 10, "runner-b", 100).unwrap();
        assert_eq!(reclaimed.len(), 2);
        assert!(store.complete_wakeup(TaskId(1), 2, Some("runner-b"), 120).unwrap());
        assert!(!store.complete_wakeup(TaskId(1), 2, Some("runner-a"), 120).unwrap());
        assert_eq!(store.wakeup(TaskId(1)).unwrap(), None);
    }

    #[test]
    fn separate_connections_cannot_claim_the_same_wakeup() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-claim-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let first = Arc::new(SqliteStore::open(&path).unwrap());
        first.schedule_wakeup(Wakeup { task_id: TaskId(20), sequence: 3, wake_at_ms: 10 }).unwrap();
        let second = Arc::new(SqliteStore::open(&path).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = [(first.clone(), "runner-a"), (second.clone(), "runner-b")]
            .into_iter()
            .map(|(store, owner)| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim_due_wakeups(10, 1, owner, 100).unwrap().len()
                })
            })
            .collect();
        barrier.wait();
        let claimed: usize = threads.into_iter().map(|thread| thread.join().unwrap()).sum();
        assert_eq!(claimed, 1);
        drop(first);
        drop(second);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn old_generation_cannot_remove_a_new_wakeup() {
        let store = SqliteStore::in_memory().unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(1), sequence: 1, wake_at_ms: 10 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(1), sequence: 2, wake_at_ms: 20 }).unwrap();
        assert!(!store.complete_wakeup(TaskId(1), 1, None, 20).unwrap());
        assert_eq!(
            store.wakeup(TaskId(1)).unwrap(),
            Some(Wakeup { task_id: TaskId(1), sequence: 2, wake_at_ms: 20 })
        );
    }

    #[test]
    fn sleep_event_and_wakeup_are_committed_together() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(5);
        let stored = store
            .append_event_with_wakeup(
                id,
                event(EventKind::Sleep, "backoff", json!({"duration_ms": 50}), 100),
                150,
            )
            .unwrap();
        assert_eq!(stored.sequence, 0);
        assert_eq!(store.load_history(id).unwrap().events, vec![stored]);
        assert_eq!(
            store.wakeup(id).unwrap(),
            Some(Wakeup { task_id: id, sequence: 0, wake_at_ms: 150 })
        );
    }

    #[test]
    fn oversized_timestamp_is_rejected() {
        let store = SqliteStore::in_memory().unwrap();
        let err = store
            .append_event(TaskId(1), event(EventKind::Now, "clock", json!(0), u64::MAX))
            .unwrap_err();
        assert!(matches!(err, DurableError::IntegerRange { field: "recorded_at_ms" }));
    }

    #[test]
    fn event_key_and_payload_are_bounded() {
        let store = SqliteStore::in_memory().unwrap();
        let key_err = store
            .append_event(
                TaskId(1),
                event(EventKind::Step, &"x".repeat(MAX_EVENT_KEY_BYTES + 1), json!(0), 1),
            )
            .unwrap_err();
        assert!(matches!(key_err, DurableError::EventKeyTooLarge));
        let payload_err = store
            .append_event(
                TaskId(1),
                event(EventKind::Step, "large", json!("x".repeat(MAX_EVENT_PAYLOAD_BYTES + 1)), 1),
            )
            .unwrap_err();
        assert!(matches!(payload_err, DurableError::EventPayloadTooLarge));
        assert!(store.load_history(TaskId(1)).unwrap().events.is_empty());
    }

    #[test]
    fn cumulative_history_budget_is_enforced_before_insert() {
        let store = SqliteStore::in_memory().unwrap();
        {
            let connection = store.lock().unwrap();
            let id = task_id_bytes(TaskId(11));
            connection
                .execute(
                    "INSERT INTO durable_history_stats
                     (task_id, event_count, payload_bytes) VALUES (?1, 0, ?2)",
                    params![id.as_slice(), MAX_HISTORY_BYTES as i64],
                )
                .unwrap();
        }
        let err = store
            .append_event(TaskId(11), event(EventKind::Step, "next", json!(1), 1))
            .unwrap_err();
        assert!(matches!(err, DurableError::HistoryByteLimit));
        assert!(store.load_history(TaskId(11)).unwrap().events.is_empty());
    }

    #[test]
    fn legacy_history_over_the_event_limit_is_rejected_on_load() {
        let store = SqliteStore::in_memory().unwrap();
        let id = task_id_bytes(TaskId(12));
        {
            let mut connection = store.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            {
                let mut statement = transaction
                    .prepare(
                        "INSERT INTO durable_events
                         (task_id, sequence, kind, event_key, payload, recorded_at_ms)
                         VALUES (?1, ?2, 'step', 'legacy', 'null', 1)",
                    )
                    .unwrap();
                for sequence in 0..=MAX_HISTORY_EVENTS {
                    statement.execute(params![id.as_slice(), sequence as i64]).unwrap();
                }
            }
            transaction.commit().unwrap();
        }
        assert!(matches!(store.load_history(TaskId(12)), Err(DurableError::HistoryEventLimit)));
    }
}
