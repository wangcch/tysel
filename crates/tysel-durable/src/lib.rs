//! Durable task event history, deterministic replay, and SQLite wakeup storage.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tysel_task::TaskId;

const MAX_WAKEUP_BATCH: usize = 10_000;
const MAX_EVENT_KEY_BYTES: usize = 256;
const MAX_EVENT_PAYLOAD_BYTES: usize = 1_048_576;
const MAX_HISTORY_EVENTS: usize = 10_000;
const MAX_HISTORY_BYTES: usize = 16 * 1_048_576;
const MAX_PENDING_SIGNALS: usize = 1_000;
const MAX_LEASE_OWNER_BYTES: usize = 128;
/// Durable SQLite schema and replay-log contract supported by this runtime.
pub const DURABLE_LOG_VERSION: u32 = 1;
pub const MAX_DURABLE_PROGRAM_BYTES: usize = 1_048_576;
pub const MAX_DURABLE_PROGRAM_TOTAL_BYTES: usize = 64 * 1_048_576;
pub const MAX_DURABLE_PROGRAMS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Step,
    Effect,
    Sleep,
    Signal,
    Retry,
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
            Self::Retry => "retry",
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
            "retry" => Ok(Self::Retry),
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
    payload_json: String,
}

impl TaskEvent {
    pub fn payload_json(&self) -> &str {
        &self.payload_json
    }
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

    /// Consume through a matching future event. Composite durable operations
    /// use this to replay a recorded outcome without rerunning already
    /// completed nested boundaries. `None` leaves the cursor unchanged.
    pub fn consume_through(&mut self, kind: EventKind, key: &str) -> Option<&TaskEvent> {
        let offset = self.events[self.position..]
            .iter()
            .position(|event| event.kind == kind && event.key == key)?;
        self.position += offset + 1;
        self.events.get(self.position - 1)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalWait {
    pub task_id: TaskId,
    pub sequence: u64,
    pub signal_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableProgram {
    pub task_id: TaskId,
    pub kind: DurableProgramKind,
    pub source: String,
    pub source_sha256: [u8; 32],
    pub registered_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableProgramKind {
    Script,
    Module,
}

impl DurableProgramKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Module => "module",
        }
    }

    fn parse(raw: &str) -> Result<Self, DurableError> {
        match raw {
            "script" => Ok(Self::Script),
            "module" => Ok(Self::Module),
            _ => Err(DurableError::InvalidProgramKind(raw.into())),
        }
    }
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

    fn from_connection(mut connection: Connection) -> Result<Self, DurableError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        initialize_schema(&mut connection)?;
        Ok(Self { connection: Mutex::new(connection) })
    }

    /// Return the durable log contract version recorded in this store.
    pub fn log_version(&self) -> Result<u32, DurableError> {
        let connection = self.lock()?;
        read_log_version(&connection)?.ok_or(DurableError::InvalidLogVersion)
    }

    /// Register immutable program text for a durable task. Repeating the same
    /// registration is idempotent; changing source for an existing task is a
    /// conflict because its history may already replay against the old code.
    pub fn put_program(
        &self,
        task_id: TaskId,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        self.put_program_inner(task_id, DurableProgramKind::Script, source, registered_at_ms)
    }

    pub fn put_module(
        &self,
        task_id: TaskId,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        self.put_program_inner(task_id, DurableProgramKind::Module, source, registered_at_ms)
    }

    fn put_program_inner(
        &self,
        task_id: TaskId,
        kind: DurableProgramKind,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        validate_program_source(source)?;
        let registered_at_ms = to_sql_integer(registered_at_ms, "registered_at_ms")?;
        let id = task_id_bytes(task_id);
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = select_program(&transaction, task_id)? {
            if existing.kind != kind
                || existing.source_sha256 != digest
                || existing.source != source
            {
                return Err(DurableError::ProgramConflict { task_id });
            }
            transaction.commit()?;
            return Ok(Some(existing));
        }
        let (count, total_bytes): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(source AS BLOB))), 0)
             FROM durable_programs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if count >= MAX_DURABLE_PROGRAMS as i64 {
            return Err(DurableError::ProgramLimit);
        }
        let source_bytes =
            i64::try_from(source.len()).map_err(|_| DurableError::ProgramByteLimit)?;
        if total_bytes
            .checked_add(source_bytes)
            .is_none_or(|bytes| bytes > MAX_DURABLE_PROGRAM_TOTAL_BYTES as i64)
        {
            return Err(DurableError::ProgramByteLimit);
        }
        transaction.execute(
            "INSERT INTO durable_programs
             (task_id, program_kind, source, source_sha256, registered_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.as_slice(), kind.as_str(), source, digest.as_slice(), registered_at_ms],
        )?;
        transaction.commit()?;
        Ok(None)
    }

    pub fn program(&self, task_id: TaskId) -> Result<Option<DurableProgram>, DurableError> {
        let connection = self.lock()?;
        select_program(&connection, task_id)
    }

    pub fn load_programs(&self) -> Result<Vec<DurableProgram>, DurableError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT task_id, program_kind, source, source_sha256, registered_at_ms
             FROM durable_programs ORDER BY task_id LIMIT ?1",
        )?;
        let rows = statement.query_map(params![(MAX_DURABLE_PROGRAMS + 1) as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        decode_program_rows(rows)
    }

    /// Load only registered programs whose wakeup is currently due and not
    /// leased. This keeps an idle poll from reading and hashing the full source
    /// catalog.
    pub fn load_due_programs(&self, now_ms: u64) -> Result<Vec<DurableProgram>, DurableError> {
        self.load_due_programs_by_kind(now_ms, DurableProgramKind::Script)
    }

    pub fn load_due_programs_by_kind(
        &self,
        now_ms: u64,
        kind: DurableProgramKind,
    ) -> Result<Vec<DurableProgram>, DurableError> {
        let connection = self.lock()?;
        let now_ms = to_sql_integer(now_ms, "now_ms")?;
        let mut statement = connection.prepare(
            "SELECT p.task_id, p.program_kind, p.source, p.source_sha256, p.registered_at_ms
             FROM durable_programs AS p
             INNER JOIN durable_wakeups AS w ON w.task_id = p.task_id
             WHERE w.wake_at_ms <= ?1
               AND (w.lease_until_ms IS NULL OR w.lease_until_ms <= ?1)
               AND p.program_kind = ?2
             ORDER BY p.task_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![now_ms, kind.as_str(), (MAX_DURABLE_PROGRAMS + 1) as i64],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        decode_program_rows(rows)
    }

    pub fn remove_program(&self, task_id: TaskId) -> Result<Option<DurableProgram>, DurableError> {
        let id = task_id_bytes(task_id);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = select_program(&transaction, task_id)?;
        if existing.is_some() {
            let in_use = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM durable_events WHERE task_id = ?1)
                     OR EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = ?1)
                     OR EXISTS(SELECT 1 FROM durable_signal_inbox WHERE task_id = ?1)
                     OR EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = ?1)",
                params![id.as_slice()],
                |row| row.get::<_, bool>(0),
            )?;
            if in_use {
                return Err(DurableError::ProgramInUse { task_id });
            }
            transaction.execute(
                "DELETE FROM durable_programs WHERE task_id = ?1",
                params![id.as_slice()],
            )?;
        }
        transaction.commit()?;
        Ok(existing)
    }

    pub fn program_count(&self) -> Result<usize, DurableError> {
        let connection = self.lock()?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM durable_programs", [], |row| row.get(0))?;
        usize::try_from(count).map_err(|_| DurableError::ProgramLimit)
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

    pub fn append_event_json_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        kind: EventKind,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        let event = raw_event(kind, key, payload_json, recorded_at_ms)?;
        let recorded_at_ms = to_sql_integer(recorded_at_ms, "recorded_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = insert_event(
            &transaction,
            task_id,
            Some(expected_sequence),
            &event,
            payload_json,
            recorded_at_ms,
        )?;
        transaction.commit()?;
        stored_event(task_id, sequence, event, payload_json.into())
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
        stored_event(task_id, sequence, event, payload)
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

    pub fn append_event_json_with_wakeup_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
        wake_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        let event = raw_event(EventKind::Sleep, key, payload_json, recorded_at_ms)?;
        let recorded_at_ms = to_sql_integer(recorded_at_ms, "recorded_at_ms")?;
        let wake_at_ms = to_sql_integer(wake_at_ms, "wake_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = insert_event(
            &transaction,
            task_id,
            Some(expected_sequence),
            &event,
            payload_json,
            recorded_at_ms,
        )?;
        upsert_wakeup(&transaction, task_id, sequence, wake_at_ms)?;
        transaction.commit()?;
        stored_event(task_id, sequence, event, payload_json.into())
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
        stored_event(task_id, sequence, event, payload)
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
                payload_json: payload,
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

    /// Lease one exact due task. This lets a dispatcher resolve a task program
    /// before claiming, so unknown tasks cannot starve runnable registered work.
    pub fn claim_wakeup(
        &self,
        task_id: TaskId,
        now_ms: u64,
        lease_owner: &str,
        lease_duration_ms: u64,
    ) -> Result<Option<WakeupClaim>, DurableError> {
        validate_lease_owner(lease_owner)?;
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let lease_until_sql = to_sql_integer(lease_until_ms, "lease_until_ms")?;
        let id = task_id_bytes(task_id);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let wakeup = transaction
            .query_row(
                "SELECT sequence, wake_at_ms FROM durable_wakeups
                 WHERE task_id = ?1 AND wake_at_ms <= ?2
                   AND (lease_until_ms IS NULL OR lease_until_ms <= ?2)",
                params![id.as_slice(), now_sql],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((sequence, wake_at_ms)) = wakeup else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE durable_wakeups SET lease_owner = ?1, lease_until_ms = ?2
             WHERE task_id = ?3 AND sequence = ?4 AND wake_at_ms = ?5
               AND (lease_until_ms IS NULL OR lease_until_ms <= ?6)",
            params![lease_owner, lease_until_sql, id.as_slice(), sequence, wake_at_ms, now_sql,],
        )?;
        transaction.commit()?;
        if changed != 1 {
            return Ok(None);
        }
        Ok(Some(WakeupClaim {
            task_id,
            sequence: from_sql_integer(sequence, "sequence")?,
            wake_at_ms: from_sql_integer(wake_at_ms, "wake_at_ms")?,
            lease_owner: lease_owner.into(),
            lease_until_ms,
        }))
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

    /// Extend an exact claim token before executing work. Matching the prior
    /// lease deadline prevents an expired runner from renewing over a re-claim.
    pub fn renew_wakeup_claim(
        &self,
        claim: &WakeupClaim,
        now_ms: u64,
        lease_duration_ms: u64,
    ) -> Result<Option<WakeupClaim>, DurableError> {
        validate_lease_owner(&claim.lease_owner)?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let connection = self.lock()?;
        let id = task_id_bytes(claim.task_id);
        let changed = connection.execute(
            "UPDATE durable_wakeups SET lease_until_ms = ?1
             WHERE task_id = ?2 AND sequence = ?3 AND wake_at_ms = ?4
               AND lease_owner = ?5 AND lease_until_ms = ?6",
            params![
                to_sql_integer(lease_until_ms, "lease_until_ms")?,
                id.as_slice(),
                to_sql_integer(claim.sequence, "sequence")?,
                to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                claim.lease_owner,
                to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
            ],
        )?;
        Ok((changed == 1).then(|| WakeupClaim {
            task_id: claim.task_id,
            sequence: claim.sequence,
            wake_at_ms: claim.wake_at_ms,
            lease_owner: claim.lease_owner.clone(),
            lease_until_ms,
        }))
    }

    /// Release an exact claim without removing its wakeup generation.
    pub fn release_wakeup_claim(&self, claim: &WakeupClaim) -> Result<bool, DurableError> {
        validate_lease_owner(&claim.lease_owner)?;
        let connection = self.lock()?;
        let id = task_id_bytes(claim.task_id);
        let changed = connection.execute(
            "UPDATE durable_wakeups SET lease_owner = NULL, lease_until_ms = NULL
             WHERE task_id = ?1 AND sequence = ?2 AND wake_at_ms = ?3
               AND lease_owner = ?4 AND lease_until_ms = ?5",
            params![
                id.as_slice(),
                to_sql_integer(claim.sequence, "sequence")?,
                to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                claim.lease_owner,
                to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
            ],
        )?;
        Ok(changed == 1)
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

    pub fn signal_wait(&self, task_id: TaskId) -> Result<Option<SignalWait>, DurableError> {
        let connection = self.lock()?;
        let id = task_id_bytes(task_id);
        connection
            .query_row(
                "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = ?1",
                params![id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(|(sequence, signal_name)| {
                Ok(SignalWait {
                    task_id,
                    sequence: from_sql_integer(sequence, "sequence")?,
                    signal_name,
                })
            })
            .transpose()
    }

    /// Persist a signal and make a matching suspended task immediately
    /// claimable. Signals sent before a task starts waiting remain in FIFO order.
    pub fn send_signal(
        &self,
        task_id: TaskId,
        signal_name: &str,
        payload: &Value,
        sent_at_ms: u64,
    ) -> Result<u64, DurableError> {
        validate_signal_name(signal_name)?;
        let payload = serde_json::to_string(payload)?;
        if payload.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(DurableError::EventPayloadTooLarge);
        }
        let sent_at_ms = to_sql_integer(sent_at_ms, "sent_at_ms")?;
        let id = task_id_bytes(task_id);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (pending_count, pending_bytes) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(
                 length(CAST(signal_name AS BLOB)) + length(CAST(payload AS BLOB))
             ), 0)
             FROM durable_signal_inbox WHERE task_id = ?1",
            params![id.as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let signal_bytes =
            signal_name.len().checked_add(payload.len()).ok_or(DurableError::SignalInboxLimit)?;
        if pending_count >= MAX_PENDING_SIGNALS as i64
            || pending_bytes
                .checked_add(
                    i64::try_from(signal_bytes).map_err(|_| DurableError::SignalInboxLimit)?,
                )
                .is_none_or(|bytes| bytes > MAX_HISTORY_BYTES as i64)
        {
            return Err(DurableError::SignalInboxLimit);
        }
        transaction.execute(
            "INSERT INTO durable_signal_inbox (task_id, signal_name, payload, sent_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![id.as_slice(), signal_name, payload, sent_at_ms],
        )?;
        let signal_id = transaction.last_insert_rowid();
        let wait_sequence = transaction
            .query_row(
                "SELECT sequence FROM durable_signal_waits
                 WHERE task_id = ?1 AND signal_name = ?2",
                params![id.as_slice(), signal_name],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(sequence) = wait_sequence {
            transaction.execute(
                "INSERT INTO durable_wakeups
                     (task_id, sequence, wake_at_ms, lease_owner, lease_until_ms)
                 VALUES (?1, ?2, ?3, NULL, NULL)
                 ON CONFLICT(task_id) DO NOTHING",
                params![id.as_slice(), sequence, sent_at_ms],
            )?;
        }
        transaction.commit()?;
        from_sql_integer(signal_id, "signal_id")
    }

    /// Atomically consume the oldest matching signal and append its replay
    /// event. If no signal exists, register the boundary as suspended.
    pub fn poll_signal(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        signal_name: &str,
        now_ms: u64,
        claim: Option<&WakeupClaim>,
    ) -> Result<Option<TaskEvent>, DurableError> {
        validate_signal_name(signal_name)?;
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        let id = task_id_bytes(task_id);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual_sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence) + 1, 0)
             FROM durable_events WHERE task_id = ?1",
            params![id.as_slice()],
            |row| row.get(0),
        )?;
        let actual_sequence = from_sql_integer(actual_sequence, "sequence")?;
        if actual_sequence != expected_sequence {
            return Err(DurableError::HistoryConflict {
                expected: expected_sequence,
                actual: actual_sequence,
            });
        }
        let existing_wait = transaction
            .query_row(
                "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = ?1",
                params![id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((wait_sequence, wait_name)) = &existing_wait
            && (from_sql_integer(*wait_sequence, "sequence")? != expected_sequence
                || wait_name != signal_name)
        {
            return Err(DurableError::SignalWaitConflict);
        }
        let queued = transaction
            .query_row(
                "SELECT id, payload, sent_at_ms FROM durable_signal_inbox
                 WHERE task_id = ?1 AND signal_name = ?2 ORDER BY id LIMIT 1",
                params![id.as_slice(), signal_name],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()?;
        let Some((signal_id, payload_json, sent_at_ms)) = queued else {
            register_signal_wait(&transaction, task_id, expected_sequence, signal_name)?;
            transaction.commit()?;
            return Ok(None);
        };

        if let Some(claim) = claim {
            if existing_wait.is_none()
                || claim.task_id != task_id
                || claim.sequence != expected_sequence
            {
                return Err(DurableError::WakeupClaimRequired);
            }
            let changed = transaction.execute(
                "DELETE FROM durable_wakeups
                 WHERE task_id = ?1 AND sequence = ?2 AND wake_at_ms = ?3
                   AND lease_owner = ?4 AND lease_until_ms = ?5
                   AND lease_until_ms > ?6",
                params![
                    id.as_slice(),
                    to_sql_integer(claim.sequence, "sequence")?,
                    to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                    claim.lease_owner,
                    to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                    now_sql,
                ],
            )?;
            if changed != 1 {
                return Err(DurableError::WakeupClaimRequired);
            }
        } else {
            if existing_wait.is_some() {
                return Err(DurableError::WakeupClaimRequired);
            }
            let has_wakeup = transaction
                .query_row(
                    "SELECT 1 FROM durable_wakeups WHERE task_id = ?1",
                    params![id.as_slice()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if has_wakeup {
                return Err(DurableError::WakeupClaimRequired);
            }
        }

        let payload: Value = serde_json::from_str(&payload_json)?;
        let event = NewEvent {
            kind: EventKind::Signal,
            key: signal_name.into(),
            payload,
            recorded_at_ms: from_sql_integer(sent_at_ms, "sent_at_ms")?,
        };
        let sequence = insert_event(
            &transaction,
            task_id,
            Some(expected_sequence),
            &event,
            &payload_json,
            sent_at_ms,
        )?;
        transaction
            .execute("DELETE FROM durable_signal_inbox WHERE id = ?1", params![signal_id])?;
        transaction.execute(
            "DELETE FROM durable_signal_waits
             WHERE task_id = ?1 AND sequence = ?2 AND signal_name = ?3",
            params![id.as_slice(), sequence, signal_name],
        )?;
        transaction.commit()?;
        stored_event(task_id, sequence, event, payload_json).map(Some)
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
    #[error("signal name must be 1..={MAX_EVENT_KEY_BYTES} bytes")]
    InvalidSignalName,
    #[error("task is already suspended on a different signal boundary")]
    SignalWaitConflict,
    #[error("an active wakeup claim is required to consume this signal")]
    WakeupClaimRequired,
    #[error(
        "pending signal inbox exceeds {MAX_PENDING_SIGNALS} signals or {MAX_HISTORY_BYTES} bytes"
    )]
    SignalInboxLimit,
    #[error("durable program must be 1..={MAX_DURABLE_PROGRAM_BYTES} bytes")]
    InvalidProgram,
    #[error("durable program catalog exceeds {MAX_DURABLE_PROGRAMS} tasks")]
    ProgramLimit,
    #[error("durable program catalog exceeds {MAX_DURABLE_PROGRAM_TOTAL_BYTES} total bytes")]
    ProgramByteLimit,
    #[error("durable program for task {task_id} is already registered with different source")]
    ProgramConflict { task_id: TaskId },
    #[error("durable program for task {task_id} still has persisted task state")]
    ProgramInUse { task_id: TaskId },
    #[error("unknown durable program kind {0:?}")]
    InvalidProgramKind(String),
    #[error("durable program digest is invalid")]
    InvalidProgramDigest,
    #[error("durable log version metadata is missing or invalid")]
    InvalidLogVersion,
    #[error("durable log version {found} is newer than this runtime supports ({supported})")]
    UnsupportedLogVersion { found: u32, supported: u32 },
}

fn validate_program_source(source: &str) -> Result<(), DurableError> {
    if source.is_empty() || source.len() > MAX_DURABLE_PROGRAM_BYTES {
        return Err(DurableError::InvalidProgram);
    }
    Ok(())
}

fn select_program(
    connection: &Connection,
    task_id: TaskId,
) -> Result<Option<DurableProgram>, DurableError> {
    let id = task_id_bytes(task_id);
    connection
        .query_row(
            "SELECT task_id, program_kind, source, source_sha256, registered_at_ms
             FROM durable_programs WHERE task_id = ?1",
            params![id.as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?
        .map(|(task_id, kind, source, digest, registered_at_ms)| {
            decode_program(task_id, kind, source, digest, registered_at_ms)
        })
        .transpose()
}

fn decode_program(
    task_id: Vec<u8>,
    kind: String,
    source: String,
    digest: Vec<u8>,
    registered_at_ms: i64,
) -> Result<DurableProgram, DurableError> {
    validate_program_source(&source)?;
    let source_sha256: [u8; 32] =
        digest.try_into().map_err(|_| DurableError::InvalidProgramDigest)?;
    let actual: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    if source_sha256 != actual {
        return Err(DurableError::InvalidProgramDigest);
    }
    Ok(DurableProgram {
        task_id: task_id_from_bytes(&task_id)?,
        kind: DurableProgramKind::parse(&kind)?,
        source,
        source_sha256,
        registered_at_ms: from_sql_integer(registered_at_ms, "registered_at_ms")?,
    })
}

fn decode_program_rows<M>(rows: M) -> Result<Vec<DurableProgram>, DurableError>
where
    M: IntoIterator<Item = Result<(Vec<u8>, String, String, Vec<u8>, i64), rusqlite::Error>>,
{
    let mut programs = Vec::new();
    let mut total_bytes = 0usize;
    for row in rows {
        if programs.len() >= MAX_DURABLE_PROGRAMS {
            return Err(DurableError::ProgramLimit);
        }
        let (task_id, kind, source, digest, registered_at_ms) = row?;
        validate_program_source(&source)?;
        total_bytes =
            total_bytes.checked_add(source.len()).ok_or(DurableError::ProgramByteLimit)?;
        if total_bytes > MAX_DURABLE_PROGRAM_TOTAL_BYTES {
            return Err(DurableError::ProgramByteLimit);
        }
        programs.push(decode_program(task_id, kind, source, digest, registered_at_ms)?);
    }
    Ok(programs)
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

fn raw_event(
    kind: EventKind,
    key: String,
    payload_json: &str,
    recorded_at_ms: u64,
) -> Result<NewEvent, DurableError> {
    if key.len() > MAX_EVENT_KEY_BYTES {
        return Err(DurableError::EventKeyTooLarge);
    }
    if payload_json.len() > MAX_EVENT_PAYLOAD_BYTES {
        return Err(DurableError::EventPayloadTooLarge);
    }
    Ok(NewEvent { kind, key, payload: serde_json::from_str(payload_json)?, recorded_at_ms })
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

fn validate_signal_name(name: &str) -> Result<(), DurableError> {
    if name.is_empty() || name.len() > MAX_EVENT_KEY_BYTES {
        return Err(DurableError::InvalidSignalName);
    }
    Ok(())
}

fn register_signal_wait(
    transaction: &Transaction<'_>,
    task_id: TaskId,
    sequence: u64,
    signal_name: &str,
) -> Result<(), DurableError> {
    let id = task_id_bytes(task_id);
    let existing = transaction
        .query_row(
            "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = ?1",
            params![id.as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((existing_sequence, existing_name)) = existing {
        if from_sql_integer(existing_sequence, "sequence")? != sequence
            || existing_name != signal_name
        {
            return Err(DurableError::SignalWaitConflict);
        }
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO durable_signal_waits (task_id, sequence, signal_name)
         VALUES (?1, ?2, ?3)",
        params![id.as_slice(), to_sql_integer(sequence, "sequence")?, signal_name],
    )?;
    Ok(())
}

fn initialize_schema(connection: &mut Connection) -> Result<(), DurableError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(found) = read_log_version(&transaction)?
        && found > DURABLE_LOG_VERSION
    {
        return Err(DurableError::UnsupportedLogVersion { found, supported: DURABLE_LOG_VERSION });
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS tysel_durable_metadata (
             key TEXT PRIMARY KEY,
             value INTEGER NOT NULL CHECK (value >= 0)
         );
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
         );
         CREATE TABLE IF NOT EXISTS durable_signal_inbox (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             task_id BLOB NOT NULL,
             signal_name TEXT NOT NULL,
             payload TEXT NOT NULL,
             sent_at_ms INTEGER NOT NULL CHECK (sent_at_ms >= 0)
         );
         CREATE TABLE IF NOT EXISTS durable_signal_waits (
             task_id BLOB PRIMARY KEY,
             sequence INTEGER NOT NULL CHECK (sequence >= 0),
             signal_name TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS durable_programs (
             task_id BLOB PRIMARY KEY,
             program_kind TEXT NOT NULL DEFAULT 'script'
                 CHECK (program_kind IN ('script', 'module')),
             source TEXT NOT NULL,
             source_sha256 BLOB NOT NULL CHECK (length(source_sha256) = 32),
             registered_at_ms INTEGER NOT NULL CHECK (registered_at_ms >= 0)
         );",
    )?;
    migrate_program_columns(&transaction)?;
    let sequence_added = migrate_wakeup_columns(&transaction)?;
    if sequence_added {
        transaction.execute_batch(
            "UPDATE durable_wakeups
             SET sequence = COALESCE((
                 SELECT MAX(sequence) FROM durable_events
                 WHERE durable_events.task_id = durable_wakeups.task_id
                   AND kind = 'sleep'
             ), sequence);",
        )?;
    }
    transaction.execute_batch(
        "DROP INDEX IF EXISTS durable_wakeups_due;
         CREATE INDEX durable_wakeups_due
             ON durable_wakeups (wake_at_ms, lease_until_ms, task_id);
         CREATE INDEX IF NOT EXISTS durable_signal_inbox_task
             ON durable_signal_inbox (task_id, signal_name, id);
         INSERT OR REPLACE INTO durable_history_stats
             (task_id, event_count, payload_bytes)
         SELECT task_id, COUNT(*), COALESCE(SUM(
             length(CAST(payload AS BLOB)) + length(CAST(event_key AS BLOB))
         ), 0)
         FROM durable_events GROUP BY task_id;",
    )?;
    transaction.execute(
        "INSERT INTO tysel_durable_metadata (key, value)
         VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![i64::from(DURABLE_LOG_VERSION)],
    )?;
    transaction.commit()?;
    Ok(())
}

fn read_log_version(connection: &Connection) -> Result<Option<u32>, DurableError> {
    let metadata_exists = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'tysel_durable_metadata'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !metadata_exists {
        return Ok(None);
    }
    let raw = connection
        .query_row(
            "SELECT value FROM tysel_durable_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(DurableError::InvalidLogVersion)?;
    u32::try_from(raw).map(Some).map_err(|_| DurableError::InvalidLogVersion)
}

fn migrate_wakeup_columns(connection: &Connection) -> Result<bool, DurableError> {
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(durable_wakeups)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let sequence_added = !columns.iter().any(|column| column == "sequence");
    if sequence_added {
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
    Ok(sequence_added)
}

fn migrate_program_columns(connection: &Connection) -> Result<(), DurableError> {
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(durable_programs)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if !columns.iter().any(|column| column == "program_kind") {
        connection.execute_batch(
            "ALTER TABLE durable_programs
             ADD COLUMN program_kind TEXT NOT NULL DEFAULT 'script'
             CHECK (program_kind IN ('script', 'module'))",
        )?;
    }
    Ok(())
}

fn stored_event(
    task_id: TaskId,
    sequence: i64,
    event: NewEvent,
    payload_json: String,
) -> Result<TaskEvent, DurableError> {
    Ok(TaskEvent {
        task_id,
        sequence: from_sql_integer(sequence, "sequence")?,
        kind: event.kind,
        key: event.key,
        payload: event.payload,
        recorded_at_ms: event.recorded_at_ms,
        payload_json,
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
    fn program_catalog_is_bounded_immutable_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-programs-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = TaskId(70);
        let source = "(async () => 42)()";
        {
            let store = SqliteStore::open(&path).unwrap();
            assert!(matches!(store.put_program(id, "", 1), Err(DurableError::InvalidProgram)));
            assert_eq!(store.put_program(id, source, 10).unwrap(), None);
            let existing = store.put_program(id, source, 20).unwrap().unwrap();
            assert_eq!(existing.registered_at_ms, 10);
            assert!(matches!(
                store.put_program(id, "(async () => 43)()", 20),
                Err(DurableError::ProgramConflict { task_id }) if task_id == id
            ));
            assert_eq!(store.program_count().unwrap(), 1);
        }
        let store = SqliteStore::open(&path).unwrap();
        let programs = store.load_programs().unwrap();
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].task_id, id);
        assert_eq!(programs[0].kind, DurableProgramKind::Script);
        assert_eq!(programs[0].source, source);
        assert_eq!(programs[0].source_sha256, Sha256::digest(source.as_bytes()).as_slice());
        assert_eq!(store.remove_program(id).unwrap(), Some(programs[0].clone()));
        assert!(store.program(id).unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn program_catalog_rejects_a_corrupted_digest() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(71);
        store.put_program(id, "42", 1).unwrap();
        let bytes = task_id_bytes(id);
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE durable_programs SET source_sha256 = zeroblob(32) WHERE task_id = ?1",
                params![bytes.as_slice()],
            )
            .unwrap();
        assert!(matches!(store.program(id), Err(DurableError::InvalidProgramDigest)));
    }

    #[test]
    fn program_in_use_cannot_be_removed_or_rebound() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(72);
        store.put_program(id, "old", 1).unwrap();
        store.append_event(id, event(EventKind::Step, "used", json!(true), 2)).unwrap();
        assert!(matches!(
            store.remove_program(id),
            Err(DurableError::ProgramInUse { task_id }) if task_id == id
        ));
        assert!(matches!(
            store.put_program(id, "new", 3),
            Err(DurableError::ProgramConflict { task_id }) if task_id == id
        ));
        assert_eq!(store.program(id).unwrap().unwrap().source, "old");
    }

    #[test]
    fn due_program_query_excludes_idle_and_leased_tasks() {
        let store = SqliteStore::in_memory().unwrap();
        let due = TaskId(73);
        let idle = TaskId(74);
        let leased = TaskId(75);
        for id in [due, idle, leased] {
            store.put_program(id, &format!("program-{id}"), 1).unwrap();
        }
        store.schedule_wakeup(Wakeup { task_id: due, sequence: 0, wake_at_ms: 10 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: idle, sequence: 0, wake_at_ms: 20 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: leased, sequence: 0, wake_at_ms: 10 }).unwrap();
        store.claim_wakeup(leased, 10, "runner", 100).unwrap().unwrap();

        let module = TaskId(76);
        store.put_module(module, "export default () => 1", 1).unwrap();
        store.schedule_wakeup(Wakeup { task_id: module, sequence: 0, wake_at_ms: 10 }).unwrap();

        let programs = store.load_due_programs(10).unwrap();
        assert_eq!(programs.iter().map(|program| program.task_id).collect::<Vec<_>>(), vec![due]);
        let modules = store.load_due_programs_by_kind(10, DurableProgramKind::Module).unwrap();
        assert_eq!(modules.iter().map(|program| program.task_id).collect::<Vec<_>>(), vec![module]);
    }

    #[test]
    fn existing_program_schema_is_migrated_as_script() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-program-migrate-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let task_id = TaskId(77);
        let source = "42";
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE durable_programs (
                         task_id BLOB PRIMARY KEY,
                         source TEXT NOT NULL,
                         source_sha256 BLOB NOT NULL,
                         registered_at_ms INTEGER NOT NULL
                     );",
                )
                .unwrap();
            let id = task_id_bytes(task_id);
            connection
                .execute(
                    "INSERT INTO durable_programs VALUES (?1, ?2, ?3, 9)",
                    params![id.as_slice(), source, digest.as_slice()],
                )
                .unwrap();
        }

        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    SqliteStore::open(path)
                })
            })
            .collect();
        barrier.wait();
        let mut stores: Vec<_> =
            handles.into_iter().map(|handle| handle.join().unwrap().unwrap()).collect();
        let store = stores.pop().unwrap();
        let program = store.program(task_id).unwrap().unwrap();
        assert_eq!(store.log_version().unwrap(), DURABLE_LOG_VERSION);
        assert_eq!(program.kind, DurableProgramKind::Script);
        assert_eq!(program.source, source);
        drop(store);
        drop(stores);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn newer_durable_log_is_rejected_without_schema_changes() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-future-version-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE tysel_durable_metadata (
                         key TEXT PRIMARY KEY,
                         value INTEGER NOT NULL
                     );
                     INSERT INTO tysel_durable_metadata VALUES ('schema_version', 2);",
                )
                .unwrap();
        }

        let error = match SqliteStore::open(&path) {
            Ok(_) => panic!("future durable log must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            DurableError::UnsupportedLogVersion { found: 2, supported: DURABLE_LOG_VERSION }
        ));
        let connection = Connection::open(&path).unwrap();
        let events_table_exists: bool = connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'durable_events'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!events_table_exists);
        drop(connection);
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
                    payload_json: "1234".into(),
                },
                TaskEvent {
                    task_id: TaskId(1),
                    sequence: 1,
                    kind: EventKind::Step,
                    key: "next".into(),
                    payload: json!({"ok": true}),
                    recorded_at_ms: 1235,
                    payload_json: r#"{"ok":true}"#.into(),
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
    fn claim_renewal_and_release_require_the_exact_token() {
        let store = SqliteStore::in_memory().unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(4), sequence: 2, wake_at_ms: 10 }).unwrap();
        let original = store.claim_due_wakeups(10, 1, "runner", 100).unwrap().pop().unwrap();
        let renewed = store.renew_wakeup_claim(&original, 50, 100).unwrap().unwrap();
        assert_eq!(renewed.lease_until_ms, 150);
        assert!(store.renew_wakeup_claim(&original, 60, 100).unwrap().is_none());
        assert!(!store.release_wakeup_claim(&original).unwrap());
        assert!(store.release_wakeup_claim(&renewed).unwrap());
        assert_eq!(store.wakeup(TaskId(4)).unwrap().unwrap().sequence, 2);
        assert_eq!(store.claim_due_wakeups(60, 1, "other", 100).unwrap().len(), 1);
    }

    #[test]
    fn exact_claim_leaves_other_due_tasks_available() {
        let store = SqliteStore::in_memory().unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(40), sequence: 1, wake_at_ms: 10 }).unwrap();
        store.schedule_wakeup(Wakeup { task_id: TaskId(41), sequence: 2, wake_at_ms: 10 }).unwrap();
        assert_eq!(store.claim_wakeup(TaskId(41), 9, "runner", 100).unwrap(), None);
        let claim = store.claim_wakeup(TaskId(41), 10, "runner", 100).unwrap().unwrap();
        assert_eq!(claim.task_id, TaskId(41));
        assert!(store.claim_wakeup(TaskId(41), 10, "other", 100).unwrap().is_none());
        let remaining = store.claim_due_wakeups(10, 10, "other", 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, TaskId(40));
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
    fn signal_sent_before_wait_is_consumed_in_fifo_order() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(30);
        store.send_signal(id, "approval", &json!({"order": 1}), 10).unwrap();
        store.send_signal(id, "approval", &json!({"order": 2}), 11).unwrap();

        let first = store.poll_signal(id, 0, "approval", 12, None).unwrap().unwrap();
        let second = store.poll_signal(id, 1, "approval", 13, None).unwrap().unwrap();
        assert_eq!(first.payload, json!({"order": 1}));
        assert_eq!(second.payload, json!({"order": 2}));
        assert_eq!(store.load_history(id).unwrap().events, vec![first, second]);
        assert_eq!(store.signal_wait(id).unwrap(), None);
        assert_eq!(store.wakeup(id).unwrap(), None);
    }

    #[test]
    fn waiting_signal_is_woken_and_requires_its_claim() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(31);
        assert_eq!(store.poll_signal(id, 0, "approval", 10, None).unwrap(), None);
        assert_eq!(
            store.signal_wait(id).unwrap(),
            Some(SignalWait { task_id: id, sequence: 0, signal_name: "approval".into() })
        );
        store.send_signal(id, "ignored", &json!(false), 11).unwrap();
        assert_eq!(store.wakeup(id).unwrap(), None);
        store.send_signal(id, "approval", &json!({"ok": true}), 12).unwrap();
        let wakeup = store.wakeup(id).unwrap().expect("signal wakeup");
        assert_eq!(wakeup.sequence, 0);
        let claim = store.claim_due_wakeups(12, 1, "signal-runner", 100).unwrap().pop().unwrap();
        assert!(matches!(
            store.poll_signal(id, 0, "ignored", 12, Some(&claim)),
            Err(DurableError::SignalWaitConflict)
        ));
        assert!(matches!(
            store.poll_signal(id, 0, "approval", 12, None),
            Err(DurableError::WakeupClaimRequired)
        ));
        let event = store.poll_signal(id, 0, "approval", 12, Some(&claim)).unwrap().unwrap();
        assert_eq!(event.kind, EventKind::Signal);
        assert_eq!(event.payload, json!({"ok": true}));
        assert_eq!(store.signal_wait(id).unwrap(), None);
        assert_eq!(store.wakeup(id).unwrap(), None);
    }

    #[test]
    fn reopening_does_not_rewrite_a_signal_wakeup_generation() {
        let path = std::env::temp_dir().join(format!(
            "tysel-durable-signal-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = TaskId(32);
        {
            let store = SqliteStore::open(&path).unwrap();
            store.append_event(id, event(EventKind::Sleep, "old", json!(null), 1)).unwrap();
            assert_eq!(store.poll_signal(id, 1, "next", 2, None).unwrap(), None);
            store.send_signal(id, "next", &json!("ready"), 3).unwrap();
            assert_eq!(store.wakeup(id).unwrap().unwrap().sequence, 1);
        }
        let reopened = SqliteStore::open(&path).unwrap();
        assert_eq!(reopened.wakeup(id).unwrap().unwrap().sequence, 1);
        let claim = reopened
            .claim_due_wakeups(3, 1, "restarted-runner", 100)
            .unwrap()
            .pop()
            .expect("signal claim after reopen");
        let event = reopened.poll_signal(id, 1, "next", 3, Some(&claim)).unwrap().unwrap();
        assert_eq!(event.payload, json!("ready"));
        assert_eq!(reopened.wakeup(id).unwrap(), None);
        drop(reopened);
        let _ = std::fs::remove_file(path);
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
    fn pending_signal_inbox_is_bounded() {
        let store = SqliteStore::in_memory().unwrap();
        let id = task_id_bytes(TaskId(33));
        {
            let mut connection = store.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            {
                let mut statement = transaction
                    .prepare(
                        "INSERT INTO durable_signal_inbox
                         (task_id, signal_name, payload, sent_at_ms)
                         VALUES (?1, 'queued', 'null', 1)",
                    )
                    .unwrap();
                for _ in 0..MAX_PENDING_SIGNALS {
                    statement.execute(params![id.as_slice()]).unwrap();
                }
            }
            transaction.commit().unwrap();
        }
        assert!(matches!(
            store.send_signal(TaskId(33), "queued", &json!(null), 2),
            Err(DurableError::SignalInboxLimit)
        ));
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
