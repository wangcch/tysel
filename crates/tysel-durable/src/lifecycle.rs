use super::*;

pub(super) fn initialize(tx: &Transaction<'_>) -> Result<(), DurableError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_completions (
            task_id BLOB PRIMARY KEY,
            next_sequence INTEGER NOT NULL CHECK (next_sequence >= 0),
            result_json TEXT NOT NULL,
            completed_at_ms INTEGER NOT NULL CHECK (completed_at_ms >= 0)
         );
         CREATE INDEX IF NOT EXISTS durable_completions_retention
             ON durable_completions (completed_at_ms, task_id);
         CREATE TABLE IF NOT EXISTS durable_program_stats (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            active_count INTEGER NOT NULL CHECK (active_count >= 0),
            active_bytes INTEGER NOT NULL CHECK (active_bytes >= 0)
         );",
    )?;
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_program_stats WHERE singleton = 1)",
        [],
        |row| row.get::<_, bool>(0),
    )? {
        tx.execute("INSERT INTO durable_program_stats SELECT 1, COUNT(*), COALESCE(SUM(length(CAST(source AS BLOB))), 0) FROM durable_programs p WHERE NOT EXISTS (SELECT 1 FROM durable_completions c WHERE c.task_id = p.task_id)", [])?;
    }
    Ok(())
}

pub(super) fn ensure_open(connection: &Connection, task_id: TaskId) -> Result<(), DurableError> {
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_completions WHERE task_id = ?1)",
        params![task_id_bytes(task_id).as_slice()],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(DurableError::TaskCompleted { task_id });
    }
    Ok(())
}

/// Validate at the terminal transaction boundary, never with a caller's stale clock.
/// The unleased low-level API preserves its explicit timestamp contract.
pub(super) fn completion_time(
    task_id: TaskId,
    timestamp: u64,
    lease_until_ms: Option<u64>,
) -> Result<u64, DurableError> {
    let Some(until) = lease_until_ms else {
        return Ok(timestamp);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_millis();
    if now >= u128::from(until) {
        return Err(DurableError::TaskSuspended { task_id });
    }
    Ok(now as u64)
}

impl SqliteStore {
    pub fn completion(&self, task_id: TaskId) -> Result<Option<TaskCompletion>, DurableError> {
        let connection = self.lock()?;
        let row = connection.query_row(
            "SELECT next_sequence, result_json, completed_at_ms FROM durable_completions WHERE task_id = ?1",
            params![task_id_bytes(task_id).as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        ).optional()?;
        row.map(|(sequence, json, time)| {
            Ok(TaskCompletion {
                task_id,
                next_sequence: from_sql_integer(sequence, "next_sequence")?,
                value: serde_json::from_str(&json)?,
                completed_at_ms: from_sql_integer(time, "completed_at_ms")?,
            })
        })
        .transpose()
    }

    /// Commit completion only after every boundary is consumed and no suspension remains.
    /// History and program identity remain available until explicit retention cleanup.
    pub fn complete_task(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        value: &Value,
        completed_at_ms: u64,
    ) -> Result<bool, DurableError> {
        self.finish_task(task_id, expected_sequence, value, completed_at_ms, None)
    }

    pub(super) fn finish_task(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        value: &Value,
        completed_at_ms: u64,
        lease_until_ms: Option<u64>,
    ) -> Result<bool, DurableError> {
        let json = serde_json::to_string(value)?;
        if json.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(DurableError::EventPayloadTooLarge);
        }
        let id = task_id_bytes(task_id);
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((sequence, stored)) = tx
            .query_row(
                "SELECT next_sequence, result_json FROM durable_completions WHERE task_id = ?1",
                params![id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if sequence != to_sql_integer(expected_sequence, "next_sequence")?
                || serde_json::from_str::<Value>(&stored)? != *value
            {
                return Err(DurableError::TaskCompleted { task_id });
            }
            return Ok(true);
        }
        let Some(source_bytes) = tx
            .query_row(
                "SELECT length(CAST(source AS BLOB)) FROM durable_programs WHERE task_id = ?1",
                params![id.as_slice()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let actual: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence) + 1, 0) FROM durable_events WHERE task_id = ?1",
            params![id.as_slice()],
            |row| row.get(0),
        )?;
        if from_sql_integer(actual, "next_sequence")? != expected_sequence {
            return Err(DurableError::HistoryConflict {
                expected: expected_sequence,
                actual: actual as u64,
            });
        }
        let suspended: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = ?1) OR EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = ?1)",
            params![id.as_slice()], |row| row.get(0),
        )?;
        if suspended {
            return Err(DurableError::TaskSuspended { task_id });
        }
        let completed_at_ms = completion_time(task_id, completed_at_ms, lease_until_ms)?;
        tx.execute("INSERT INTO durable_completions (task_id, next_sequence, result_json, completed_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![id.as_slice(), actual, json, to_sql_integer(completed_at_ms, "completed_at_ms")?])?;
        tx.execute("UPDATE durable_program_stats SET active_count = active_count - 1, active_bytes = active_bytes - ?1 WHERE singleton = 1", params![source_bytes])?;
        completion_time(task_id, completed_at_ms, lease_until_ms)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn prune_completed(&self, before_ms: u64, limit: usize) -> Result<usize, DurableError> {
        if limit == 0 {
            return Ok(0);
        }
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT c.task_id, COALESCE(h.payload_bytes, 0) + COALESCE(length(CAST(p.source AS BLOB)), 0) + length(CAST(c.result_json AS BLOB))
                   + COALESCE((SELECT SUM(length(CAST(s.signal_name AS BLOB)) + length(CAST(s.payload AS BLOB))) FROM durable_signal_inbox s WHERE s.task_id = c.task_id), 0)
                 FROM durable_completions c
                 LEFT JOIN durable_programs p ON p.task_id = c.task_id
                 LEFT JOIN durable_history_stats h ON h.task_id = c.task_id
                 WHERE c.completed_at_ms < ?1 ORDER BY c.completed_at_ms, c.task_id LIMIT ?2",
            )?;
            statement
                .query_map(
                    params![to_sql_integer(before_ms, "before_ms")?, limit.min(100) as i64],
                    |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut deleted = 0;
        let mut bytes = 0u64;
        for (id, retained_bytes) in candidates {
            if deleted > 0 && bytes + retained_bytes as u64 > MAX_HISTORY_BYTES as u64 {
                break;
            }
            let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = ?1) OR EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = ?1)", params![&id], |row| row.get(0))?;
            if live {
                continue;
            }
            // Names are static SQL identifiers; no external input enters this SQL.
            for table in [
                "durable_programs",
                "durable_events",
                "durable_history_stats",
                "durable_signal_inbox",
                "durable_completions",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE task_id = ?1"), params![&id])?;
            }
            bytes += retained_bytes as u64;
            deleted += 1;
        }
        tx.commit()?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leased_completion_preserves_idempotence_and_uses_transaction_time() {
        let store = SqliteStore::in_memory().unwrap();
        let id = TaskId(801);
        store.put_program(id, "42", 0).unwrap();
        assert!(matches!(
            store.complete_task_before(id, 0, &Value::from(42), 0),
            Err(DurableError::TaskSuspended { .. })
        ));
        assert!(store.completion(id).unwrap().is_none());
        let before = completion_time(id, 0, Some(u64::MAX)).unwrap();
        assert!(store.complete_task_before(id, 0, &Value::from(42), u64::MAX).unwrap());
        assert!(store.completion(id).unwrap().unwrap().completed_at_ms >= before);
        assert!(store.complete_task_before(id, 0, &Value::from(42), 0).unwrap());
        assert!(matches!(
            store.complete_task_before(id, 0, &Value::from(43), 0),
            Err(DurableError::TaskCompleted { .. })
        ));
        assert_eq!(store.program_count().unwrap(), 0);
    }

    #[test]
    fn completion_releases_quota_and_retention_preserves_live_history() {
        let store = SqliteStore::in_memory().unwrap();
        let done = TaskId(1);
        let live = TaskId(2);
        store.put_module(done, "export default async () => 1", 0).unwrap();
        store.append_event_json_at(done, 0, EventKind::Step, "input".into(), "null", 0).unwrap();
        assert!(matches!(
            store.complete_task(done, 0, &Value::from(7), 10),
            Err(DurableError::HistoryConflict { .. })
        ));
        store.schedule_wakeup(Wakeup { task_id: done, sequence: 0, wake_at_ms: 1 }).unwrap();
        assert!(matches!(
            store.complete_task(done, 1, &Value::from(7), 10),
            Err(DurableError::TaskSuspended { .. })
        ));
        let claim = store.claim_wakeup(done, 1, "owner", 20).unwrap().unwrap();
        assert!(store.complete_wakeup(done, 0, Some(&claim.lease_owner), 2).unwrap());
        assert!(store.complete_task(done, 1, &Value::from(7), 10).unwrap());
        assert!(store.complete_task(done, 1, &Value::from(7), 11).unwrap());
        assert_eq!(store.program_count().unwrap(), 0);
        assert!(store.load_programs().unwrap().is_empty());
        assert_eq!(store.completion(done).unwrap().unwrap().value, Value::from(7));
        assert_eq!(store.load_history(done).unwrap().events.len(), 1);
        assert!(matches!(
            store.send_signal(done, "late", &Value::Null, 12),
            Err(DurableError::TaskCompleted { .. })
        ));
        assert!(matches!(
            store.put_module(done, "export default async () => 1", 12),
            Err(DurableError::TaskCompleted { .. })
        ));
        assert!(matches!(
            store.append_event_json_at(done, 1, EventKind::Step, "late".into(), "null", 12),
            Err(DurableError::TaskCompleted { .. })
        ));
        assert!(matches!(
            store.poll_signal(done, 1, "late", 12, None),
            Err(DurableError::TaskCompleted { .. })
        ));
        store.put_module(live, "export default async () => 2", 0).unwrap();
        store.append_event_json_with_wakeup_at(live, 0, "sleep".into(), "null", 0, 1).unwrap();
        assert_eq!(store.prune_completed(10, 100).unwrap(), 0);
        assert_eq!(store.prune_completed(11, 0).unwrap(), 0);
        assert_eq!(store.prune_completed(11, 1).unwrap(), 1);
        assert!(store.completion(done).unwrap().is_none());
        assert!(store.program(done).unwrap().is_none());
        assert!(store.load_history(done).unwrap().events.is_empty());
        assert_eq!(store.program_count().unwrap(), 1);
        assert!(store.wakeup(live).unwrap().is_some());
        assert_eq!(store.load_history(live).unwrap().events.len(), 1);
    }

    #[test]
    fn completed_tasks_can_exceed_the_active_catalog_limit() {
        let store = SqliteStore::in_memory().unwrap();
        for n in 0..MAX_DURABLE_PROGRAMS + 1 {
            let id = TaskId(n as u128);
            store.put_program(id, "1", 0).unwrap();
            store.complete_task(id, 0, &Value::Null, 1).unwrap();
        }
        assert_eq!(store.program_count().unwrap(), 0);
        assert_eq!(store.prune_completed(2, usize::MAX).unwrap(), 100);
        assert!(store.completion(TaskId(100)).unwrap().is_some());
    }

    #[test]
    fn completion_and_wakeup_are_serial_across_sqlite_connections() {
        use std::sync::{Arc, Barrier};
        let path = std::env::temp_dir().join(format!(
            "tysel-completion-race-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        let other = Arc::new(SqliteStore::open(&path).unwrap());
        for n in 0..16 {
            let id = TaskId(n);
            store.put_program(id, "1", 0).unwrap();
            let barrier = Arc::new(Barrier::new(3));
            let finish = {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.complete_task(id, 0, &Value::Null, 1)
                })
            };
            let suspend = {
                let other = other.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    other.schedule_wakeup(Wakeup { task_id: id, sequence: 0, wake_at_ms: 1 })
                })
            };
            barrier.wait();
            match (finish.join().unwrap(), suspend.join().unwrap()) {
                (Ok(true), Err(DurableError::TaskCompleted { .. }))
                | (Err(DurableError::TaskSuspended { .. }), Ok(())) => {}
                other => panic!("non-serial completion and suspension: {other:?}"),
            }
        }
        drop(other);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn retention_budget_counts_source_and_result_bytes() {
        let store = SqliteStore::in_memory().unwrap();
        let source = "x".repeat(MAX_DURABLE_PROGRAM_BYTES);
        let value = Value::String("v".repeat(MAX_EVENT_PAYLOAD_BYTES - 2));
        for n in 0..9 {
            store.put_program(TaskId(n), &source, 0).unwrap();
            store.complete_task(TaskId(n), 0, &value, 1).unwrap();
        }
        assert_eq!(store.prune_completed(2, 100).unwrap(), 8);
        assert!(store.completion(TaskId(8)).unwrap().is_some());
    }

    #[test]
    fn v1_migration_preserves_unclassified_tasks_and_accounting_on_reopen() {
        let path = std::env::temp_dir().join(format!(
            "tysel-lifecycle-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        {
            let store = SqliteStore::open(&path).unwrap();
            store.put_program(TaskId(1), "1", 0).unwrap();
            let connection = store.lock().unwrap();
            connection.execute_batch("DROP TABLE durable_completions; DROP TABLE durable_program_stats; UPDATE tysel_durable_metadata SET value = 1 WHERE key = 'schema_version';").unwrap();
        }
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.log_version().unwrap(), 2);
            assert_eq!(store.program_count().unwrap(), 1);
            assert_eq!(store.prune_completed(u64::MAX / 2, 100).unwrap(), 0);
            store.complete_task(TaskId(1), 0, &Value::from(1), 10).unwrap();
        }
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.program_count().unwrap(), 0);
            assert_eq!(store.completion(TaskId(1)).unwrap().unwrap().value, Value::from(1));
            store.put_program(TaskId(2), "2", 11).unwrap();
            assert_eq!(store.program_count().unwrap(), 1);
        }
        std::fs::remove_file(path).unwrap();
    }
}
