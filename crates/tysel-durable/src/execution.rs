use super::*;

/// Fences one execution independently of its currently suspended boundary.
#[derive(Debug, Clone)]
pub struct ExecutionClaim {
    pub task_id: TaskId,
    pub owner: String,
    pub generation: u64,
    pub lease_until_ms: u64,
    pub wakeup: Option<WakeupClaim>,
}

/// Store-wide identity. Applications should namespace keys by tenant and operation.
pub fn admission_task_id(key: &str) -> Result<TaskId, DurableError> {
    validate_key(key)?;
    let mut hash = Sha256::new();
    hash.update(b"tysel:admission:v1:");
    hash.update(key.as_bytes());
    let digest = hash.finalize();
    Ok(TaskId(u128::from_be_bytes(digest[..16].try_into().expect("digest prefix"))))
}

pub(super) fn owner_token(owner: &str) -> Result<String, DurableError> {
    validate_lease_owner(owner)?;
    let mut entropy = [0u8; 32];
    getrandom::fill(&mut entropy).map_err(std::io::Error::other)?;
    let mut hash = Sha256::new();
    hash.update(owner.as_bytes());
    hash.update(entropy);
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn now() -> Result<u64, DurableError> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_millis();
    u64::try_from(ms).map_err(|_| DurableError::IntegerRange { field: "time" })
}
pub(super) fn validate_key(key: &str) -> Result<(), DurableError> {
    if key.is_empty() || key.len() > 256 {
        return Err(DurableError::InvalidIdempotencyKey);
    }
    Ok(())
}
pub(super) fn fingerprint(source: &str, input: &str) -> Result<(String, String), DurableError> {
    validate_program_source(source)?;
    if input.len() > MAX_EVENT_PAYLOAD_BYTES {
        return Err(DurableError::EventPayloadTooLarge);
    }
    let input = serde_json::to_string(&serde_json::from_str::<Value>(input)?)?;
    let mut hash = Sha256::new();
    hash.update(source.as_bytes());
    hash.update([0]);
    hash.update(input.as_bytes());
    Ok((format!("{:x}", hash.finalize()), input))
}

pub(super) fn initialize(tx: &Transaction<'_>) -> Result<(), DurableError> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS durable_executions (
        task_id BLOB PRIMARY KEY,
        state TEXT NOT NULL CHECK (state IN ('ready','running','suspended','failed','completed')),
        generation INTEGER NOT NULL DEFAULT 0,
        owner TEXT, lease_until_ms INTEGER,
        admission_key TEXT, request_hash TEXT
    );
    CREATE INDEX IF NOT EXISTS durable_executions_due ON durable_executions(state, lease_until_ms, task_id);
    CREATE TABLE IF NOT EXISTS durable_signal_receipts (
        task_id BLOB NOT NULL, request_key TEXT NOT NULL, signal_name TEXT NOT NULL,
        payload TEXT NOT NULL, signal_id INTEGER NOT NULL, PRIMARY KEY(task_id, request_key)
    );")?;
    Ok(())
}

impl SqliteStore {
    pub(super) fn guard_execution(
        &self,
        tx: &Connection,
        task: TaskId,
    ) -> Result<(), DurableError> {
        let Some(claim) = &self.execution else {
            return Ok(());
        };
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM durable_executions WHERE task_id=?1 AND state='running' AND owner=?2 AND generation=?3 AND lease_until_ms > ?4)",
            params![task_id_bytes(task).as_slice(), claim.owner, to_sql_integer(claim.generation,"generation")?, to_sql_integer(now()?,"time")?], |r|r.get(0))?;
        if task != claim.task_id || !valid {
            return Err(DurableError::ExecutionLeaseLost);
        }
        Ok(())
    }

    pub(super) fn admit(
        &self,
        task: TaskId,
        key: &str,
        source: &str,
        input: &str,
        time: u64,
    ) -> Result<(), DurableError> {
        validate_key(key)?;
        let (hash, input) = fingerprint(source, input)?;
        let id = task_id_bytes(task);
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((old_key, old_hash)) = tx
            .query_row(
                "SELECT admission_key, request_hash FROM durable_executions WHERE task_id=?1",
                params![id.as_slice()],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?
        {
            if old_key.as_deref() != Some(key) || old_hash.as_deref() != Some(&hash) {
                return Err(DurableError::AdmissionConflict);
            }
            return Ok(());
        }
        if select_program(&tx, task)?.is_some() {
            return Err(DurableError::AdmissionConflict);
        }
        let (count, bytes): (i64, i64) = tx.query_row(
            "SELECT active_count, active_bytes FROM durable_program_stats WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count >= MAX_DURABLE_PROGRAMS as i64 {
            return Err(DurableError::ProgramLimit);
        }
        if bytes + source.len() as i64 > MAX_DURABLE_PROGRAM_TOTAL_BYTES as i64 {
            return Err(DurableError::ProgramByteLimit);
        }
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        tx.execute("INSERT INTO durable_programs(task_id,program_kind,source,source_sha256,registered_at_ms) VALUES (?1,'module',?2,?3,?4)",params![id.as_slice(),source,digest.as_slice(),to_sql_integer(time,"time")?])?;
        tx.execute("UPDATE durable_program_stats SET active_count=active_count+1, active_bytes=active_bytes+?1 WHERE singleton=1",params![source.len() as i64])?;
        let event = raw_event(EventKind::Step, "$tysel:task-input".into(), &input, time)?;
        insert_event(&tx, task, Some(0), &event, &input, to_sql_integer(time, "time")?)?;
        tx.execute("INSERT INTO durable_executions(task_id,state,admission_key,request_hash) VALUES (?1,'ready',?2,?3)",params![id.as_slice(),key,hash])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn claim_run(
        &self,
        task: TaskId,
        owner: &str,
        duration: u64,
    ) -> Result<Option<ExecutionClaim>, DurableError> {
        let token = owner_token(owner)?;
        let owner = token.as_str();
        let id = task_id_bytes(task);
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_completions WHERE task_id=?1)",
            params![id.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(None);
        }
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_programs WHERE task_id=?1)",
            params![id.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(None);
        }
        let time = now()?;
        let until = time
            .checked_add(duration)
            .filter(|u| *u > time)
            .ok_or(DurableError::ExecutionLeaseLost)?;
        let row = tx
            .query_row(
                "SELECT state,generation,lease_until_ms FROM durable_executions WHERE task_id=?1",
                params![id.as_slice()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)),
            )
            .optional()?;
        if let Some((state, _, lease)) = &row
            && (state == "completed" || state == "failed" || lease.is_some_and(|u| u > time as i64))
        {
            return Ok(None);
        }
        let wake = tx
            .query_row(
                "SELECT sequence,wake_at_ms,lease_until_ms FROM durable_wakeups WHERE task_id=?1",
                params![id.as_slice()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)),
            )
            .optional()?;
        if wake.as_ref().is_some_and(|(_, at, lease)| {
            *at > time as i64 || lease.is_some_and(|u| u > time as i64)
        }) {
            return Ok(None);
        }
        if wake.is_none()
            && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id=?1)",
                params![id.as_slice()],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Ok(None);
        }
        let generation = row
            .map_or(Ok(1), |(_, g, _)| g.checked_add(1).ok_or(DurableError::ExecutionLeaseLost))?;
        tx.execute("INSERT INTO durable_executions(task_id,state,generation,owner,lease_until_ms) VALUES (?1,'running',?2,?3,?4) ON CONFLICT(task_id) DO UPDATE SET state='running',generation=excluded.generation,owner=excluded.owner,lease_until_ms=excluded.lease_until_ms",params![id.as_slice(),generation,owner,to_sql_integer(until,"lease")?])?;
        let wakeup = if let Some((seq, at, _)) = wake {
            tx.execute(
                "UPDATE durable_wakeups SET lease_owner=?2,lease_until_ms=?3 WHERE task_id=?1",
                params![id.as_slice(), owner, to_sql_integer(until, "lease")?],
            )?;
            Some(WakeupClaim {
                task_id: task,
                sequence: seq as u64,
                wake_at_ms: at as u64,
                lease_owner: owner.into(),
                lease_until_ms: until,
            })
        } else {
            None
        };
        if now()? >= until {
            return Ok(None);
        }
        tx.commit()?;
        Ok(Some(ExecutionClaim {
            task_id: task,
            owner: owner.into(),
            generation: generation as u64,
            lease_until_ms: until,
            wakeup,
        }))
    }

    pub(super) fn release_run(
        &self,
        claim: &ExecutionClaim,
        failed: bool,
    ) -> Result<(), DurableError> {
        let scoped = Self { connection: self.connection.clone(), execution: Some(claim.clone()) };
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        scoped.guard_execution(&tx, claim.task_id)?;
        let id = task_id_bytes(claim.task_id);
        tx.execute(
            "UPDATE durable_wakeups SET lease_owner=NULL,lease_until_ms=NULL WHERE task_id=?1",
            params![id.as_slice()],
        )?;
        scoped.guard_execution(&tx, claim.task_id)?;
        tx.execute("UPDATE durable_executions SET state=?2,owner=NULL,lease_until_ms=NULL WHERE task_id=?1",params![id.as_slice(),if failed {"failed"}else{"suspended"}])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_admission_rolls_back_program_input_and_quota() {
        let store = SqliteStore::in_memory().unwrap();
        store.lock().unwrap().execute_batch("CREATE TRIGGER fail_input BEFORE INSERT ON durable_events BEGIN SELECT json('malformed'); END;").unwrap();
        let id = admission_task_id("rollback").unwrap();
        assert!(
            store
                .admit_module(
                    id,
                    "rollback",
                    "export default async () => 42",
                    "null",
                    now().unwrap()
                )
                .is_err()
        );
        assert_eq!(store.program_count().unwrap(), 0);
        assert!(store.program(id).unwrap().is_none());
        assert!(store.load_history(id).unwrap().events.is_empty());
        store.lock().unwrap().execute_batch("DROP TRIGGER fail_input").unwrap();
        store
            .admit_module(id, "rollback", "export default async () => 42", "null", now().unwrap())
            .unwrap();
        assert_eq!(store.program_count().unwrap(), 1);
    }
    #[test]
    fn concurrent_admission_has_one_input_and_execution() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let id = admission_task_id("concurrent").unwrap();
        let mut workers = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            workers.push(std::thread::spawn(move || {
                store.admit_module(
                    id,
                    "concurrent",
                    "export default async () => 42",
                    "null",
                    now().unwrap(),
                )
            }));
        }
        for w in workers {
            w.join().unwrap().unwrap();
        }
        assert_eq!(store.program_count().unwrap(), 1);
        assert_eq!(store.load_history(id).unwrap().events.len(), 1);
        let claim = store.claim_execution(id, "owner", 10_000).unwrap().unwrap();
        assert!(store.claim_execution(id, "owner", 10_000).unwrap().is_none());
        let scoped = store.execution_store(&claim);
        scoped.complete_task_before(id, 1, &Value::Null, claim.lease_until_ms).unwrap();
        store.prune_completed(u64::MAX / 2, 100).unwrap();
        store
            .admit_module(id, "concurrent", "export default async () => 42", "null", now().unwrap())
            .unwrap();
        let replacement = store.claim_execution(id, "owner", 10_000).unwrap().unwrap();
        assert_ne!(
            claim.owner, replacement.owner,
            "new incarnation must not reuse fencing authority"
        );
        assert!(matches!(
            scoped.append_event_json_at(
                id,
                1,
                EventKind::Effect,
                "late".into(),
                "null",
                now().unwrap()
            ),
            Err(DurableError::ExecutionLeaseLost)
        ));
    }
}
