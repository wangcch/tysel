use native_tls::TlsConnector;
use postgres::{Client, GenericClient, NoTls, Transaction};
use postgres_native_tls::MakeTlsConnector;
use r2d2_postgres::PostgresConnectionManager;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tysel_task::TaskId;

use super::{
    DURABLE_LOG_VERSION, DurableError, DurableProgram, DurableProgramKind, DurableStore, EventKind,
    History, MAX_DURABLE_PROGRAM_TOTAL_BYTES, MAX_DURABLE_PROGRAMS, MAX_EVENT_PAYLOAD_BYTES,
    MAX_HISTORY_BYTES, MAX_HISTORY_EVENTS, MAX_PENDING_SIGNALS, NewEvent, SignalWait, TaskEvent,
    Wakeup, WakeupClaim, from_sql_integer, raw_event, stored_event, task_id_bytes,
    task_id_from_bytes, to_sql_integer, validate_lease_owner, validate_program_source,
    validate_signal_name,
};

const MAX_WAKEUP_BATCH: usize = 10_000;
const DEFAULT_POOL_SIZE: u32 = 16;
const MAX_POOL_SIZE: u32 = 128;
pub const POSTGRES_URL_ENV: &str = "TYSEL_DURABLE_POSTGRES_URL";

enum Pool {
    Plain(r2d2::Pool<PostgresConnectionManager<NoTls>>),
    Tls(r2d2::Pool<PostgresConnectionManager<MakeTlsConnector>>),
}

/// Production durable store backed by PostgreSQL.
///
/// Connections are pooled per process while correctness is coordinated in the
/// database. Every per-task mutation locks an exact task row, and due wakeup
/// acquisition uses `FOR UPDATE SKIP LOCKED` so multiple schedulers can safely
/// share one store.
pub struct PostgresStore {
    pool: Pool,
}

impl PostgresStore {
    /// Connect from the host-only production secret. The URL is never read
    /// from a manifest or TAP and is not retained in diagnostics.
    pub fn connect_from_env() -> Result<Self, DurableError> {
        let url = std::env::var(POSTGRES_URL_ENV).ok().filter(|url| !url.is_empty()).ok_or(
            DurableError::PostgresConfiguration("TYSEL_DURABLE_POSTGRES_URL is missing or empty"),
        )?;
        Self::connect(&url)
    }

    pub fn connect(url: &str) -> Result<Self, DurableError> {
        Self::connect_with_pool_size(url, DEFAULT_POOL_SIZE)
    }

    pub fn connect_with_pool_size(url: &str, pool_size: u32) -> Result<Self, DurableError> {
        if pool_size == 0 || pool_size > MAX_POOL_SIZE {
            return Err(DurableError::PostgresPool(format!(
                "pool size must be 1..={MAX_POOL_SIZE}"
            )));
        }
        let config = url
            .parse::<postgres::Config>()
            .map_err(|_| DurableError::PostgresConfiguration("connection URL is invalid"))?;
        let use_tls = !matches!(config.get_ssl_mode(), postgres::config::SslMode::Disable);
        let pool = if use_tls {
            let connector = TlsConnector::builder()
                .build()
                .map_err(|error| DurableError::PostgresPool(error.to_string()))?;
            let manager = PostgresConnectionManager::new(config, MakeTlsConnector::new(connector));
            Pool::Tls(r2d2::Pool::builder().max_size(pool_size).build(manager).map_err(pool_error)?)
        } else {
            let manager = PostgresConnectionManager::new(config, NoTls);
            Pool::Plain(
                r2d2::Pool::builder().max_size(pool_size).build(manager).map_err(pool_error)?,
            )
        };
        let store = Self { pool };
        store.with_client(initialize_schema)?;
        Ok(store)
    }

    fn with_client<T>(
        &self,
        operation: impl FnOnce(&mut Client) -> Result<T, DurableError>,
    ) -> Result<T, DurableError> {
        match &self.pool {
            Pool::Plain(pool) => {
                let mut client = pool.get().map_err(pool_error)?;
                operation(&mut client)
            }
            Pool::Tls(pool) => {
                let mut client = pool.get().map_err(pool_error)?;
                operation(&mut client)
            }
        }
    }
}

fn pool_error(error: impl std::fmt::Display) -> DurableError {
    DurableError::PostgresPool(error.to_string())
}

fn initialize_schema(client: &mut Client) -> Result<(), DurableError> {
    let mut tx = client.transaction()?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS tysel_durable_metadata (
             key TEXT PRIMARY KEY,
             value BIGINT NOT NULL CHECK (value >= 0)
         );
         CREATE TABLE IF NOT EXISTS durable_task_locks (
             task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16)
         );
         CREATE TABLE IF NOT EXISTS durable_events (
             task_id BYTEA NOT NULL CHECK (octet_length(task_id) = 16),
             sequence BIGINT NOT NULL CHECK (sequence >= 0),
             kind TEXT NOT NULL,
             event_key TEXT NOT NULL,
             payload TEXT NOT NULL,
             recorded_at_ms BIGINT NOT NULL CHECK (recorded_at_ms >= 0),
             PRIMARY KEY (task_id, sequence)
         );
         CREATE TABLE IF NOT EXISTS durable_wakeups (
             task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16),
             sequence BIGINT NOT NULL CHECK (sequence >= 0),
             wake_at_ms BIGINT NOT NULL CHECK (wake_at_ms >= 0),
             lease_owner TEXT,
             lease_until_ms BIGINT CHECK (lease_until_ms >= 0)
         );
         CREATE TABLE IF NOT EXISTS durable_history_stats (
             task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16),
             event_count BIGINT NOT NULL CHECK (event_count >= 0),
             payload_bytes BIGINT NOT NULL CHECK (payload_bytes >= 0)
         );
         CREATE TABLE IF NOT EXISTS durable_signal_inbox (
             id BIGSERIAL PRIMARY KEY,
             task_id BYTEA NOT NULL CHECK (octet_length(task_id) = 16),
             signal_name TEXT NOT NULL,
             payload TEXT NOT NULL,
             sent_at_ms BIGINT NOT NULL CHECK (sent_at_ms >= 0)
         );
         CREATE TABLE IF NOT EXISTS durable_signal_waits (
             task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16),
             sequence BIGINT NOT NULL CHECK (sequence >= 0),
             signal_name TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS durable_programs (
             task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16),
             program_kind TEXT NOT NULL CHECK (program_kind IN ('script', 'module')),
             source TEXT NOT NULL,
             source_sha256 BYTEA NOT NULL CHECK (octet_length(source_sha256) = 32),
             registered_at_ms BIGINT NOT NULL CHECK (registered_at_ms >= 0)
         );
         CREATE INDEX IF NOT EXISTS durable_wakeups_due
             ON durable_wakeups (wake_at_ms, lease_until_ms, task_id);
         CREATE INDEX IF NOT EXISTS durable_signal_inbox_task
             ON durable_signal_inbox (task_id, signal_name, id);",
    )?;
    let found = tx
        .query_opt("SELECT value FROM tysel_durable_metadata WHERE key = 'schema_version'", &[])?
        .map(|row| row.get::<_, i64>(0));
    if let Some(found) = found {
        let found = u32::try_from(found).map_err(|_| DurableError::InvalidLogVersion)?;
        if found > DURABLE_LOG_VERSION {
            return Err(DurableError::UnsupportedLogVersion {
                found,
                supported: DURABLE_LOG_VERSION,
            });
        }
    }
    tx.execute(
        "INSERT INTO tysel_durable_metadata (key, value) VALUES ('schema_version', $1)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[&i64::from(DURABLE_LOG_VERSION)],
    )?;
    tx.commit()?;
    Ok(())
}

fn lock_task(tx: &mut Transaction<'_>, task_id: TaskId) -> Result<[u8; 16], DurableError> {
    let id = task_id_bytes(task_id);
    tx.execute(
        "INSERT INTO durable_task_locks (task_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&&id[..]],
    )?;
    tx.query_one(
        "SELECT task_id FROM durable_task_locks WHERE task_id = $1 FOR UPDATE",
        &[&&id[..]],
    )?;
    Ok(id)
}

fn decode_program(row: &postgres::Row) -> Result<DurableProgram, DurableError> {
    let source: String = row.get(2);
    validate_program_source(&source)?;
    let digest: Vec<u8> = row.get(3);
    let source_sha256: [u8; 32] =
        digest.try_into().map_err(|_| DurableError::InvalidProgramDigest)?;
    let actual: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    if source_sha256 != actual {
        return Err(DurableError::InvalidProgramDigest);
    }
    Ok(DurableProgram {
        task_id: task_id_from_bytes(row.get::<_, &[u8]>(0))?,
        kind: DurableProgramKind::parse(row.get::<_, &str>(1))?,
        source,
        source_sha256,
        registered_at_ms: from_sql_integer(row.get(4), "registered_at_ms")?,
    })
}

fn select_program(
    client: &mut impl GenericClient,
    task_id: TaskId,
) -> Result<Option<DurableProgram>, DurableError> {
    let id = task_id_bytes(task_id);
    client
        .query_opt(
            "SELECT task_id, program_kind, source, source_sha256, registered_at_ms
             FROM durable_programs WHERE task_id = $1",
            &[&&id[..]],
        )?
        .as_ref()
        .map(decode_program)
        .transpose()
}

fn insert_event(
    tx: &mut Transaction<'_>,
    task_id: TaskId,
    expected_sequence: u64,
    event: &NewEvent,
    payload: &str,
    recorded_at_ms: i64,
) -> Result<i64, DurableError> {
    let id = lock_task(tx, task_id)?;
    let stats = tx.query_opt(
        "SELECT event_count, payload_bytes FROM durable_history_stats WHERE task_id = $1",
        &[&&id[..]],
    )?;
    let (count, bytes) = stats.map_or((0, 0), |row| (row.get::<_, i64>(0), row.get::<_, i64>(1)));
    if count >= MAX_HISTORY_EVENTS as i64 {
        return Err(DurableError::HistoryEventLimit);
    }
    let event_bytes =
        event.key.len().checked_add(payload.len()).ok_or(DurableError::HistoryByteLimit)?;
    let next_bytes = bytes
        .checked_add(i64::try_from(event_bytes).map_err(|_| DurableError::HistoryByteLimit)?)
        .ok_or(DurableError::HistoryByteLimit)?;
    if next_bytes > MAX_HISTORY_BYTES as i64 {
        return Err(DurableError::HistoryByteLimit);
    }
    let sequence: i64 = tx
        .query_one(
            "SELECT COALESCE(MAX(sequence) + 1, 0) FROM durable_events WHERE task_id = $1",
            &[&&id[..]],
        )?
        .get(0);
    let actual = from_sql_integer(sequence, "sequence")?;
    if actual != expected_sequence {
        return Err(DurableError::HistoryConflict { expected: expected_sequence, actual });
    }
    tx.execute(
        "INSERT INTO durable_events
         (task_id, sequence, kind, event_key, payload, recorded_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[&&id[..], &sequence, &event.kind.as_str(), &event.key, &payload, &recorded_at_ms],
    )?;
    tx.execute(
        "INSERT INTO durable_history_stats (task_id, event_count, payload_bytes)
         VALUES ($1, 1, $2)
         ON CONFLICT (task_id) DO UPDATE SET
           event_count = durable_history_stats.event_count + 1,
           payload_bytes = durable_history_stats.payload_bytes + EXCLUDED.payload_bytes",
        &[&&id[..], &i64::try_from(event_bytes).map_err(|_| DurableError::HistoryByteLimit)?],
    )?;
    Ok(sequence)
}

fn upsert_wakeup(
    tx: &mut Transaction<'_>,
    task_id: TaskId,
    sequence: i64,
    wake_at_ms: i64,
) -> Result<(), DurableError> {
    let id = task_id_bytes(task_id);
    tx.execute(
        "INSERT INTO durable_wakeups
         (task_id, sequence, wake_at_ms, lease_owner, lease_until_ms)
         VALUES ($1, $2, $3, NULL, NULL)
         ON CONFLICT (task_id) DO UPDATE SET sequence = EXCLUDED.sequence,
           wake_at_ms = EXCLUDED.wake_at_ms, lease_owner = NULL, lease_until_ms = NULL",
        &[&&id[..], &sequence, &wake_at_ms],
    )?;
    Ok(())
}

impl PostgresStore {
    fn put_program_inner(
        &self,
        task_id: TaskId,
        kind: DurableProgramKind,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        validate_program_source(source)?;
        let registered_at_ms = to_sql_integer(registered_at_ms, "registered_at_ms")?;
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let id = lock_task(&mut tx, task_id)?;
            if let Some(existing) = select_program(&mut tx, task_id)? {
                if existing.kind != kind
                    || existing.source_sha256 != digest
                    || existing.source != source
                {
                    return Err(DurableError::ProgramConflict { task_id });
                }
                tx.commit()?;
                return Ok(Some(existing));
            }
            tx.query_one(
                "SELECT value FROM tysel_durable_metadata WHERE key = 'schema_version' FOR UPDATE",
                &[],
            )?;
            let row = tx.query_one(
                "SELECT COUNT(*), COALESCE(SUM(octet_length(source)), 0) FROM durable_programs",
                &[],
            )?;
            let count: i64 = row.get(0);
            let total_bytes: i64 = row.get(1);
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
            tx.execute(
                "INSERT INTO durable_programs
                 (task_id, program_kind, source, source_sha256, registered_at_ms)
                 VALUES ($1, $2, $3, $4, $5)",
                &[&&id[..], &kind.as_str(), &source, &&digest[..], &registered_at_ms],
            )?;
            tx.commit()?;
            Ok(None)
        })
    }

    fn append_validated(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        event: NewEvent,
        payload_json: &str,
        wake_at_ms: Option<u64>,
    ) -> Result<TaskEvent, DurableError> {
        let recorded_sql = to_sql_integer(event.recorded_at_ms, "recorded_at_ms")?;
        let wake_sql = wake_at_ms.map(|value| to_sql_integer(value, "wake_at_ms")).transpose()?;
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let sequence = insert_event(
                &mut tx,
                task_id,
                expected_sequence,
                &event,
                payload_json,
                recorded_sql,
            )?;
            if let Some(wake_at_ms) = wake_sql {
                upsert_wakeup(&mut tx, task_id, sequence, wake_at_ms)?;
            }
            tx.commit()?;
            stored_event(task_id, sequence, event, payload_json.into())
        })
    }
}

impl DurableStore for PostgresStore {
    fn log_version(&self) -> Result<u32, DurableError> {
        self.with_client(|client| {
            let row = client
                .query_opt(
                    "SELECT value FROM tysel_durable_metadata WHERE key = 'schema_version'",
                    &[],
                )?
                .ok_or(DurableError::InvalidLogVersion)?;
            u32::try_from(row.get::<_, i64>(0)).map_err(|_| DurableError::InvalidLogVersion)
        })
    }

    fn put_program(
        &self,
        task_id: TaskId,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        self.put_program_inner(task_id, DurableProgramKind::Script, source, registered_at_ms)
    }

    fn put_module(
        &self,
        task_id: TaskId,
        source: &str,
        registered_at_ms: u64,
    ) -> Result<Option<DurableProgram>, DurableError> {
        self.put_program_inner(task_id, DurableProgramKind::Module, source, registered_at_ms)
    }

    fn program(&self, task_id: TaskId) -> Result<Option<DurableProgram>, DurableError> {
        self.with_client(|client| select_program(client, task_id))
    }

    fn load_due_programs_by_kind(
        &self,
        now_ms: u64,
        kind: DurableProgramKind,
    ) -> Result<Vec<DurableProgram>, DurableError> {
        let now_ms = to_sql_integer(now_ms, "now_ms")?;
        self.with_client(|client| {
            let rows = client.query(
                "SELECT p.task_id, p.program_kind, p.source, p.source_sha256, p.registered_at_ms
                 FROM durable_programs p JOIN durable_wakeups w ON w.task_id = p.task_id
                 WHERE w.wake_at_ms <= $1 AND (w.lease_until_ms IS NULL OR w.lease_until_ms <= $1)
                   AND p.program_kind = $2 ORDER BY p.task_id LIMIT $3",
                &[&now_ms, &kind.as_str(), &((MAX_DURABLE_PROGRAMS + 1) as i64)],
            )?;
            if rows.len() > MAX_DURABLE_PROGRAMS {
                return Err(DurableError::ProgramLimit);
            }
            let mut programs = Vec::with_capacity(rows.len());
            let mut total = 0usize;
            for row in &rows {
                let program = decode_program(row)?;
                total = total
                    .checked_add(program.source.len())
                    .ok_or(DurableError::ProgramByteLimit)?;
                if total > MAX_DURABLE_PROGRAM_TOTAL_BYTES {
                    return Err(DurableError::ProgramByteLimit);
                }
                programs.push(program);
            }
            Ok(programs)
        })
    }

    fn remove_program(&self, task_id: TaskId) -> Result<Option<DurableProgram>, DurableError> {
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let id = lock_task(&mut tx, task_id)?;
            let existing = select_program(&mut tx, task_id)?;
            if existing.is_some() {
                let row = tx.query_one(
                    "SELECT EXISTS(SELECT 1 FROM durable_events WHERE task_id = $1)
                       OR EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = $1)
                       OR EXISTS(SELECT 1 FROM durable_signal_inbox WHERE task_id = $1)
                       OR EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = $1)",
                    &[&&id[..]],
                )?;
                if row.get::<_, bool>(0) {
                    return Err(DurableError::ProgramInUse { task_id });
                }
                tx.execute("DELETE FROM durable_programs WHERE task_id = $1", &[&&id[..]])?;
            }
            tx.commit()?;
            Ok(existing)
        })
    }

    fn program_count(&self) -> Result<usize, DurableError> {
        self.with_client(|client| {
            let count: i64 = client.query_one("SELECT COUNT(*) FROM durable_programs", &[])?.get(0);
            usize::try_from(count).map_err(|_| DurableError::ProgramLimit)
        })
    }

    fn append_event_json_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        kind: EventKind,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        let event = raw_event(kind, key, payload_json, recorded_at_ms)?;
        self.append_validated(task_id, expected_sequence, event, payload_json, None)
    }

    fn append_event_json_with_wakeup_at(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        key: String,
        payload_json: &str,
        recorded_at_ms: u64,
        wake_at_ms: u64,
    ) -> Result<TaskEvent, DurableError> {
        let event = raw_event(EventKind::Sleep, key, payload_json, recorded_at_ms)?;
        self.append_validated(task_id, expected_sequence, event, payload_json, Some(wake_at_ms))
    }

    fn load_history(&self, task_id: TaskId) -> Result<History, DurableError> {
        let id = task_id_bytes(task_id);
        self.with_client(|client| {
            let rows = client.query(
                "SELECT sequence, kind, event_key, payload, recorded_at_ms
                 FROM durable_events WHERE task_id = $1 ORDER BY sequence LIMIT $2",
                &[&&id[..], &((MAX_HISTORY_EVENTS + 1) as i64)],
            )?;
            let mut events = Vec::with_capacity(rows.len());
            let mut bytes = 0usize;
            for row in rows {
                if events.len() >= MAX_HISTORY_EVENTS {
                    return Err(DurableError::HistoryEventLimit);
                }
                let key: String = row.get(2);
                let payload_json: String = row.get(3);
                bytes = bytes
                    .checked_add(key.len())
                    .and_then(|n| n.checked_add(payload_json.len()))
                    .ok_or(DurableError::HistoryByteLimit)?;
                if bytes > MAX_HISTORY_BYTES {
                    return Err(DurableError::HistoryByteLimit);
                }
                events.push(TaskEvent {
                    task_id,
                    sequence: from_sql_integer(row.get(0), "sequence")?,
                    kind: EventKind::parse(row.get::<_, &str>(1))?,
                    key,
                    payload: serde_json::from_str(&payload_json)?,
                    recorded_at_ms: from_sql_integer(row.get(4), "recorded_at_ms")?,
                    payload_json,
                });
            }
            Ok(History { task_id, events })
        })
    }

    fn claim_due_wakeups(
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
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        let until_sql = to_sql_integer(lease_until_ms, "lease_until_ms")?;
        let limit = i64::try_from(limit.min(MAX_WAKEUP_BATCH)).expect("bounded wakeup limit");
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let rows = tx.query(
                "WITH candidates AS (
                   SELECT task_id FROM durable_wakeups
                   WHERE wake_at_ms <= $1 AND (lease_until_ms IS NULL OR lease_until_ms <= $1)
                   ORDER BY wake_at_ms, task_id FOR UPDATE SKIP LOCKED LIMIT $2
                 )
                 UPDATE durable_wakeups w SET lease_owner = $3, lease_until_ms = $4
                 FROM candidates c WHERE w.task_id = c.task_id
                 RETURNING w.task_id, w.sequence, w.wake_at_ms",
                &[&now_sql, &limit, &lease_owner, &until_sql],
            )?;
            let mut claims = Vec::with_capacity(rows.len());
            for row in rows {
                claims.push(WakeupClaim {
                    task_id: task_id_from_bytes(row.get::<_, &[u8]>(0))?,
                    sequence: from_sql_integer(row.get(1), "sequence")?,
                    wake_at_ms: from_sql_integer(row.get(2), "wake_at_ms")?,
                    lease_owner: lease_owner.into(),
                    lease_until_ms,
                });
            }
            claims.sort_unstable_by_key(|claim| (claim.wake_at_ms, claim.task_id));
            tx.commit()?;
            Ok(claims)
        })
    }

    fn claim_wakeup(
        &self,
        task_id: TaskId,
        now_ms: u64,
        lease_owner: &str,
        lease_duration_ms: u64,
    ) -> Result<Option<WakeupClaim>, DurableError> {
        validate_lease_owner(lease_owner)?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let id = task_id_bytes(task_id);
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        let until_sql = to_sql_integer(lease_until_ms, "lease_until_ms")?;
        self.with_client(|client| {
            let row = client.query_opt(
                "UPDATE durable_wakeups SET lease_owner = $1, lease_until_ms = $2
                 WHERE task_id = $3 AND wake_at_ms <= $4
                   AND (lease_until_ms IS NULL OR lease_until_ms <= $4)
                 RETURNING sequence, wake_at_ms",
                &[&lease_owner, &until_sql, &&id[..], &now_sql],
            )?;
            row.map(|row| {
                Ok(WakeupClaim {
                    task_id,
                    sequence: from_sql_integer(row.get(0), "sequence")?,
                    wake_at_ms: from_sql_integer(row.get(1), "wake_at_ms")?,
                    lease_owner: lease_owner.into(),
                    lease_until_ms,
                })
            })
            .transpose()
        })
    }

    fn complete_wakeup(
        &self,
        task_id: TaskId,
        sequence: u64,
        lease_owner: Option<&str>,
        now_ms: u64,
    ) -> Result<bool, DurableError> {
        let id = task_id_bytes(task_id);
        let sequence = to_sql_integer(sequence, "sequence")?;
        let now_ms = to_sql_integer(now_ms, "now_ms")?;
        self.with_client(|client| {
            let changed = if let Some(owner) = lease_owner {
                validate_lease_owner(owner)?;
                client.execute(
                    "DELETE FROM durable_wakeups WHERE task_id = $1 AND sequence = $2
                     AND lease_owner = $3 AND lease_until_ms > $4",
                    &[&&id[..], &sequence, &owner, &now_ms],
                )?
            } else {
                client.execute(
                    "DELETE FROM durable_wakeups WHERE task_id = $1 AND sequence = $2
                     AND lease_owner IS NULL",
                    &[&&id[..], &sequence],
                )?
            };
            Ok(changed == 1)
        })
    }

    fn claim_is_active(&self, claim: &WakeupClaim, now_ms: u64) -> Result<bool, DurableError> {
        let id = task_id_bytes(claim.task_id);
        self.with_client(|client| {
            Ok(client
                .query_opt(
                    "SELECT 1 FROM durable_wakeups WHERE task_id = $1 AND sequence = $2
                     AND wake_at_ms = $3 AND lease_owner = $4 AND lease_until_ms = $5
                     AND lease_until_ms > $6",
                    &[
                        &&id[..],
                        &to_sql_integer(claim.sequence, "sequence")?,
                        &to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                        &claim.lease_owner,
                        &to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                        &to_sql_integer(now_ms, "now_ms")?,
                    ],
                )?
                .is_some())
        })
    }

    fn renew_wakeup_claim(
        &self,
        claim: &WakeupClaim,
        now_ms: u64,
        lease_duration_ms: u64,
    ) -> Result<Option<WakeupClaim>, DurableError> {
        validate_lease_owner(&claim.lease_owner)?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration_ms.max(1))
            .ok_or(DurableError::IntegerRange { field: "lease_until_ms" })?;
        let id = task_id_bytes(claim.task_id);
        self.with_client(|client| {
            let changed = client.execute(
                "UPDATE durable_wakeups SET lease_until_ms = $1 WHERE task_id = $2
                 AND sequence = $3 AND wake_at_ms = $4 AND lease_owner = $5
                 AND lease_until_ms = $6",
                &[
                    &to_sql_integer(lease_until_ms, "lease_until_ms")?,
                    &&id[..],
                    &to_sql_integer(claim.sequence, "sequence")?,
                    &to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                    &claim.lease_owner,
                    &to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                ],
            )?;
            Ok((changed == 1).then(|| WakeupClaim {
                task_id: claim.task_id,
                sequence: claim.sequence,
                wake_at_ms: claim.wake_at_ms,
                lease_owner: claim.lease_owner.clone(),
                lease_until_ms,
            }))
        })
    }

    fn release_wakeup_claim(&self, claim: &WakeupClaim) -> Result<bool, DurableError> {
        validate_lease_owner(&claim.lease_owner)?;
        let id = task_id_bytes(claim.task_id);
        self.with_client(|client| {
            Ok(client.execute(
                "UPDATE durable_wakeups SET lease_owner = NULL, lease_until_ms = NULL
                 WHERE task_id = $1 AND sequence = $2 AND wake_at_ms = $3
                   AND lease_owner = $4 AND lease_until_ms = $5",
                &[
                    &&id[..],
                    &to_sql_integer(claim.sequence, "sequence")?,
                    &to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                    &claim.lease_owner,
                    &to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                ],
            )? == 1)
        })
    }

    fn wakeup(&self, task_id: TaskId) -> Result<Option<Wakeup>, DurableError> {
        let id = task_id_bytes(task_id);
        self.with_client(|client| {
            client
                .query_opt(
                    "SELECT sequence, wake_at_ms FROM durable_wakeups WHERE task_id = $1",
                    &[&&id[..]],
                )?
                .map(|row| {
                    Ok(Wakeup {
                        task_id,
                        sequence: from_sql_integer(row.get(0), "sequence")?,
                        wake_at_ms: from_sql_integer(row.get(1), "wake_at_ms")?,
                    })
                })
                .transpose()
        })
    }

    fn signal_wait(&self, task_id: TaskId) -> Result<Option<SignalWait>, DurableError> {
        let id = task_id_bytes(task_id);
        self.with_client(|client| {
            client
                .query_opt(
                    "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = $1",
                    &[&&id[..]],
                )?
                .map(|row| {
                    Ok(SignalWait {
                        task_id,
                        sequence: from_sql_integer(row.get(0), "sequence")?,
                        signal_name: row.get(1),
                    })
                })
                .transpose()
        })
    }

    fn send_signal(
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
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let id = lock_task(&mut tx, task_id)?;
            let row = tx.query_one(
                "SELECT COUNT(*), COALESCE(SUM(octet_length(signal_name) + octet_length(payload)), 0)
                 FROM durable_signal_inbox WHERE task_id = $1",
                &[&&id[..]],
            )?;
            let count: i64 = row.get(0);
            let bytes: i64 = row.get(1);
            let signal_bytes = signal_name.len().checked_add(payload.len()).ok_or(DurableError::SignalInboxLimit)?;
            if count >= MAX_PENDING_SIGNALS as i64
                || bytes
                    .checked_add(i64::try_from(signal_bytes).map_err(|_| DurableError::SignalInboxLimit)?)
                    .is_none_or(|value| value > MAX_HISTORY_BYTES as i64)
            {
                return Err(DurableError::SignalInboxLimit);
            }
            let signal_id: i64 = tx
                .query_one(
                    "INSERT INTO durable_signal_inbox (task_id, signal_name, payload, sent_at_ms)
                     VALUES ($1, $2, $3, $4) RETURNING id",
                    &[&&id[..], &signal_name, &payload, &sent_at_ms],
                )?
                .get(0);
            if let Some(row) = tx.query_opt(
                "SELECT sequence FROM durable_signal_waits WHERE task_id = $1 AND signal_name = $2",
                &[&&id[..], &signal_name],
            )? {
                let sequence: i64 = row.get(0);
                tx.execute(
                    "INSERT INTO durable_wakeups
                     (task_id, sequence, wake_at_ms, lease_owner, lease_until_ms)
                     VALUES ($1, $2, $3, NULL, NULL) ON CONFLICT (task_id) DO NOTHING",
                    &[&&id[..], &sequence, &sent_at_ms],
                )?;
            }
            tx.commit()?;
            from_sql_integer(signal_id, "signal_id")
        })
    }

    fn poll_signal(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        signal_name: &str,
        now_ms: u64,
        claim: Option<&WakeupClaim>,
    ) -> Result<Option<TaskEvent>, DurableError> {
        validate_signal_name(signal_name)?;
        let now_sql = to_sql_integer(now_ms, "now_ms")?;
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let id = lock_task(&mut tx, task_id)?;
            let actual: i64 = tx
                .query_one(
                    "SELECT COALESCE(MAX(sequence) + 1, 0) FROM durable_events WHERE task_id = $1",
                    &[&&id[..]],
                )?
                .get(0);
            let actual = from_sql_integer(actual, "sequence")?;
            if actual != expected_sequence {
                return Err(DurableError::HistoryConflict { expected: expected_sequence, actual });
            }
            let existing_wait = tx.query_opt(
                "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = $1",
                &[&&id[..]],
            )?;
            if let Some(row) = &existing_wait {
                let sequence = from_sql_integer(row.get(0), "sequence")?;
                let name: &str = row.get(1);
                if sequence != expected_sequence || name != signal_name {
                    return Err(DurableError::SignalWaitConflict);
                }
            }
            let queued = tx.query_opt(
                "SELECT id, payload, sent_at_ms FROM durable_signal_inbox
                 WHERE task_id = $1 AND signal_name = $2 ORDER BY id LIMIT 1 FOR UPDATE",
                &[&&id[..], &signal_name],
            )?;
            let Some(queued) = queued else {
                tx.execute(
                    "INSERT INTO durable_signal_waits (task_id, sequence, signal_name)
                     VALUES ($1, $2, $3) ON CONFLICT (task_id) DO NOTHING",
                    &[&&id[..], &to_sql_integer(expected_sequence, "sequence")?, &signal_name],
                )?;
                let row = tx.query_one(
                    "SELECT sequence, signal_name FROM durable_signal_waits WHERE task_id = $1",
                    &[&&id[..]],
                )?;
                if from_sql_integer(row.get(0), "sequence")? != expected_sequence
                    || row.get::<_, &str>(1) != signal_name
                {
                    return Err(DurableError::SignalWaitConflict);
                }
                tx.commit()?;
                return Ok(None);
            };

            if let Some(claim) = claim {
                if existing_wait.is_none() || claim.task_id != task_id || claim.sequence != expected_sequence {
                    return Err(DurableError::WakeupClaimRequired);
                }
                let changed = tx.execute(
                    "DELETE FROM durable_wakeups WHERE task_id = $1 AND sequence = $2
                     AND wake_at_ms = $3 AND lease_owner = $4 AND lease_until_ms = $5
                     AND lease_until_ms > $6",
                    &[
                        &&id[..],
                        &to_sql_integer(claim.sequence, "sequence")?,
                        &to_sql_integer(claim.wake_at_ms, "wake_at_ms")?,
                        &claim.lease_owner,
                        &to_sql_integer(claim.lease_until_ms, "lease_until_ms")?,
                        &now_sql,
                    ],
                )?;
                if changed != 1 {
                    return Err(DurableError::WakeupClaimRequired);
                }
            } else if existing_wait.is_some()
                || tx.query_opt("SELECT 1 FROM durable_wakeups WHERE task_id = $1", &[&&id[..]])?.is_some()
            {
                return Err(DurableError::WakeupClaimRequired);
            }

            let signal_id: i64 = queued.get(0);
            let payload_json: String = queued.get(1);
            let sent_at_ms: i64 = queued.get(2);
            let event = NewEvent {
                kind: EventKind::Signal,
                key: signal_name.into(),
                payload: serde_json::from_str(&payload_json)?,
                recorded_at_ms: from_sql_integer(sent_at_ms, "sent_at_ms")?,
            };
            let sequence = insert_event(
                &mut tx,
                task_id,
                expected_sequence,
                &event,
                &payload_json,
                sent_at_ms,
            )?;
            tx.execute("DELETE FROM durable_signal_inbox WHERE id = $1", &[&signal_id])?;
            tx.execute(
                "DELETE FROM durable_signal_waits WHERE task_id = $1 AND sequence = $2 AND signal_name = $3",
                &[&&id[..], &sequence, &signal_name],
            )?;
            tx.commit()?;
            stored_event(task_id, sequence, event, payload_json).map(Some)
        })
    }
}
