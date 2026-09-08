use super::*;

pub(super) fn initialize(tx: &mut Transaction<'_>) -> Result<(), DurableError> {
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS durable_completions (
            task_id BYTEA PRIMARY KEY CHECK (octet_length(task_id) = 16),
            next_sequence BIGINT NOT NULL CHECK (next_sequence >= 0),
            result_json TEXT NOT NULL,
            completed_at_ms BIGINT NOT NULL CHECK (completed_at_ms >= 0)
         );
         CREATE INDEX IF NOT EXISTS durable_completions_retention ON durable_completions (completed_at_ms, task_id);
         CREATE TABLE IF NOT EXISTS durable_program_stats (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            active_count BIGINT NOT NULL CHECK (active_count >= 0),
            active_bytes BIGINT NOT NULL CHECK (active_bytes >= 0)
         );",
    )?;
    if tx
        .query_opt("SELECT singleton FROM durable_program_stats WHERE singleton = 1", &[])?
        .is_none()
    {
        tx.execute(
            "INSERT INTO durable_program_stats SELECT 1, COUNT(*), COALESCE(SUM(octet_length(source)), 0)
             FROM durable_programs p WHERE NOT EXISTS (SELECT 1 FROM durable_completions c WHERE c.task_id = p.task_id)
             ON CONFLICT (singleton) DO NOTHING", &[],
        )?;
    }
    Ok(())
}

fn lock_for_completion(tx: &mut Transaction<'_>, id: &[u8]) -> Result<(), DurableError> {
    tx.execute(
        "INSERT INTO durable_task_locks (task_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&id],
    )?;
    tx.query_one("SELECT task_id FROM durable_task_locks WHERE task_id = $1 FOR UPDATE", &[&id])?;
    Ok(())
}

impl PostgresStore {
    pub(super) fn read_completion(
        &self,
        task_id: TaskId,
    ) -> Result<Option<TaskCompletion>, DurableError> {
        self.with_client(|client| {
            let id = task_id_bytes(task_id);
            client.query_opt("SELECT next_sequence, result_json, completed_at_ms FROM durable_completions WHERE task_id = $1", &[&&id[..]])?
                .map(|row| Ok(TaskCompletion {
                    task_id,
                    next_sequence: from_sql_integer(row.get(0), "next_sequence")?,
                    value: serde_json::from_str(row.get::<_, &str>(1))?,
                    completed_at_ms: from_sql_integer(row.get(2), "completed_at_ms")?,
                })).transpose()
        })
    }

    pub(super) fn finish_task(
        &self,
        task_id: TaskId,
        expected_sequence: u64,
        value: &Value,
        completed_at_ms: u64,
    ) -> Result<bool, DurableError> {
        let json = serde_json::to_string(value)?;
        if json.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(DurableError::EventPayloadTooLarge);
        }
        self.with_client(|client| {
            let mut tx = client.transaction()?;
            let id = task_id_bytes(task_id);
            lock_for_completion(&mut tx, &id)?;
            if let Some(row) = tx.query_opt("SELECT next_sequence, result_json FROM durable_completions WHERE task_id = $1", &[&&id[..]])? {
                if row.get::<_, i64>(0) != to_sql_integer(expected_sequence, "next_sequence")? || serde_json::from_str::<Value>(row.get::<_, &str>(1))? != *value {
                    return Err(DurableError::TaskCompleted { task_id });
                }
                return Ok(true);
            }
            let Some(program) = tx.query_opt("SELECT octet_length(source) FROM durable_programs WHERE task_id = $1", &[&&id[..]])? else { return Ok(false); };
            let source_bytes = i64::from(program.get::<_, i32>(0));
            let actual: i64 = tx.query_one("SELECT COALESCE(MAX(sequence) + 1, 0) FROM durable_events WHERE task_id = $1", &[&&id[..]])?.get(0);
            if from_sql_integer(actual, "next_sequence")? != expected_sequence {
                return Err(DurableError::HistoryConflict { expected: expected_sequence, actual: actual as u64 });
            }
            let suspended: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = $1) OR EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = $1)", &[&&id[..]])?.get(0);
            if suspended { return Err(DurableError::TaskSuspended { task_id }); }
            tx.execute("INSERT INTO durable_completions (task_id, next_sequence, result_json, completed_at_ms) VALUES ($1, $2, $3, $4)",
                &[&&id[..], &actual, &json, &to_sql_integer(completed_at_ms, "completed_at_ms")?])?;
            tx.execute("UPDATE durable_program_stats SET active_count = active_count - 1, active_bytes = active_bytes - $1 WHERE singleton = 1", &[&source_bytes])?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub(super) fn prune_finished(
        &self,
        before_ms: u64,
        limit: usize,
    ) -> Result<usize, DurableError> {
        if limit == 0 {
            return Ok(0);
        }
        self.with_client(|client| {
            let candidates = client.query(
                "SELECT c.task_id, COALESCE(h.payload_bytes, 0) + COALESCE(octet_length(p.source), 0) + octet_length(c.result_json)
                   + COALESCE((SELECT SUM(octet_length(s.signal_name) + octet_length(s.payload)) FROM durable_signal_inbox s WHERE s.task_id = c.task_id), 0)
                 FROM durable_completions c
                 LEFT JOIN durable_programs p ON p.task_id = c.task_id
                 LEFT JOIN durable_history_stats h ON h.task_id = c.task_id
                 WHERE completed_at_ms < $1 ORDER BY completed_at_ms, c.task_id LIMIT $2",
                &[&to_sql_integer(before_ms, "before_ms")?, &(limit.min(100) as i64)],
            )?;
            let mut deleted = 0;
            let mut bytes = 0u64;
            for row in candidates {
                let id: &[u8] = row.get(0);
                let retained_bytes = row.get::<_, i64>(1) as u64;
                if deleted > 0 && bytes + retained_bytes > MAX_HISTORY_BYTES as u64 { break; }
                let mut tx = client.transaction()?;
                // The same task-row lock order as writers avoids GC/finalization deadlocks.
                lock_for_completion(&mut tx, id)?;
                let eligible: bool = tx.query_one(
                    "SELECT EXISTS(SELECT 1 FROM durable_completions WHERE task_id = $1 AND completed_at_ms < $2)
                     AND NOT EXISTS(SELECT 1 FROM durable_wakeups WHERE task_id = $1)
                     AND NOT EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id = $1)",
                    &[&id, &to_sql_integer(before_ms, "before_ms")?],
                )?.get(0);
                if !eligible { continue; }
                for table in ["durable_programs", "durable_events", "durable_history_stats", "durable_signal_inbox", "durable_completions"] {
                    tx.execute(&format!("DELETE FROM {table} WHERE task_id = $1"), &[&id])?;
                }
                // Retain the small synchronization row: deleting a row that another
                // transaction already waits on would break mutual exclusion.
                tx.commit()?;
                bytes += retained_bytes;
                deleted += 1;
            }
            Ok(deleted)
        })
    }
}
