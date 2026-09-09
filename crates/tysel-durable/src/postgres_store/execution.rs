use super::*;
use crate::execution::{fingerprint, now, validate_key};

pub(super) fn initialize(tx: &mut Transaction<'_>) -> Result<(), DurableError> {
    tx.batch_execute("CREATE TABLE IF NOT EXISTS durable_executions (
        task_id BYTEA PRIMARY KEY,
        state TEXT NOT NULL CHECK (state IN ('ready','running','suspended','failed','completed')),
        generation BIGINT NOT NULL DEFAULT 0,
        owner TEXT, lease_until_ms BIGINT,
        admission_key TEXT, request_hash TEXT
    );
    CREATE INDEX IF NOT EXISTS durable_executions_due ON durable_executions(state, lease_until_ms, task_id);
    CREATE TABLE IF NOT EXISTS durable_signal_receipts (
        task_id BYTEA NOT NULL, request_key TEXT NOT NULL, signal_name TEXT NOT NULL,
        payload TEXT NOT NULL, signal_id BIGINT NOT NULL, PRIMARY KEY(task_id, request_key)
    );")?;
    Ok(())
}
impl PostgresStore {
    pub(super) fn guard_execution(
        &self,
        tx: &mut Transaction<'_>,
        task: TaskId,
    ) -> Result<(), DurableError> {
        let Some(claim) = &self.execution else {
            return Ok(());
        };
        // Share the task lock with admission, claims, completion and all event writes.
        let id = task_id_bytes(task);
        tx.query_one(
            "SELECT task_id FROM durable_task_locks WHERE task_id=$1 FOR UPDATE",
            &[&&id[..]],
        )?;
        let valid:bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM durable_executions WHERE task_id=$1 AND state='running' AND owner=$2 AND generation=$3 AND lease_until_ms > $4)",
            &[&&id[..],&claim.owner,&to_sql_integer(claim.generation,"generation")?,&to_sql_integer(now()?,"time")?])?.get(0);
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
        self.with_client(|client| {
            let mut tx=client.transaction()?;
            let id=task_id_bytes(task);
            tx.execute("INSERT INTO durable_task_locks(task_id) VALUES ($1) ON CONFLICT DO NOTHING", &[&&id[..]])?;
            tx.query_one("SELECT task_id FROM durable_task_locks WHERE task_id=$1 FOR UPDATE", &[&&id[..]])?;
            if let Some(row)=tx.query_opt("SELECT admission_key,request_hash FROM durable_executions WHERE task_id=$1", &[&&id[..]])? {
                if row.get::<_,Option<&str>>(0)!=Some(key) || row.get::<_,Option<&str>>(1)!=Some(hash.as_str()) {return Err(DurableError::AdmissionConflict);}
                return Ok(());
            }
            if select_program(&mut tx,task)?.is_some() {return Err(DurableError::AdmissionConflict);}
            let row=tx.query_one("SELECT active_count,active_bytes FROM durable_program_stats WHERE singleton=1 FOR UPDATE",&[])?;
            if row.get::<_,i64>(0)>=MAX_DURABLE_PROGRAMS as i64 {return Err(DurableError::ProgramLimit);}
            if row.get::<_,i64>(1)+source.len() as i64 > MAX_DURABLE_PROGRAM_TOTAL_BYTES as i64 {return Err(DurableError::ProgramByteLimit);}
            let digest:[u8;32]=Sha256::digest(source.as_bytes()).into();
            tx.execute("INSERT INTO durable_programs(task_id,program_kind,source,source_sha256,registered_at_ms) VALUES ($1,'module',$2,$3,$4)",&[&&id[..],&source,&&digest[..],&to_sql_integer(time,"time")?])?;
            tx.execute("UPDATE durable_program_stats SET active_count=active_count+1,active_bytes=active_bytes+$1 WHERE singleton=1",&[&(source.len() as i64)])?;
            let event=raw_event(EventKind::Step,"$tysel:task-input".into(),&input,time)?;
            insert_event(&mut tx,task,0,&event,&input,to_sql_integer(time,"time")?)?;
            tx.execute("INSERT INTO durable_executions(task_id,state,admission_key,request_hash) VALUES ($1,'ready',$2,$3)",&[&&id[..],&key,&hash])?;
            tx.commit()?; Ok(())
        })
    }
    pub(super) fn claim_run(
        &self,
        task: TaskId,
        owner: &str,
        duration: u64,
    ) -> Result<Option<ExecutionClaim>, DurableError> {
        let token = crate::execution::owner_token(owner)?;
        let owner = token.as_str();
        self.with_client(|client| {
            let mut tx=client.transaction()?;
            let id=task_id_bytes(task);
            tx.execute("INSERT INTO durable_task_locks(task_id) VALUES ($1) ON CONFLICT DO NOTHING", &[&&id[..]])?;
            // Skip a busy task instead of serializing the scheduler behind it.
            if tx.query_opt("SELECT task_id FROM durable_task_locks WHERE task_id=$1 FOR UPDATE SKIP LOCKED", &[&&id[..]])?.is_none() {return Ok(None);}
            if tx.query_one("SELECT EXISTS(SELECT 1 FROM durable_completions WHERE task_id=$1)",&[&&id[..]])?.get::<_,bool>(0) {return Ok(None);}
            if !tx.query_one("SELECT EXISTS(SELECT 1 FROM durable_programs WHERE task_id=$1)",&[&&id[..]])?.get::<_,bool>(0) {return Ok(None);}
            let time=now()?;
            let until=time.checked_add(duration).filter(|u|*u>time).ok_or(DurableError::ExecutionLeaseLost)?;
            let row=tx.query_opt("SELECT state,generation,lease_until_ms FROM durable_executions WHERE task_id=$1",&[&&id[..]])?;
            if let Some(row)=&row
                && (matches!(row.get::<_,&str>(0),"completed"|"failed") || row.get::<_,Option<i64>>(2).is_some_and(|u|u>time as i64)) {return Ok(None);}
            let wake=tx.query_opt("SELECT sequence,wake_at_ms,lease_until_ms FROM durable_wakeups WHERE task_id=$1 FOR UPDATE",&[&&id[..]])?;
            if wake.as_ref().is_some_and(|w|w.get::<_,i64>(1)>time as i64 || w.get::<_,Option<i64>>(2).is_some_and(|u|u>time as i64)) {return Ok(None);}
            if wake.is_none() && tx.query_one("SELECT EXISTS(SELECT 1 FROM durable_signal_waits WHERE task_id=$1)",&[&&id[..]])?.get::<_,bool>(0) {return Ok(None);}
            let generation=row.map_or(Ok(1),|r|r.get::<_,i64>(1).checked_add(1).ok_or(DurableError::ExecutionLeaseLost))?;
            tx.execute("INSERT INTO durable_executions(task_id,state,generation,owner,lease_until_ms) VALUES ($1,'running',$2,$3,$4) ON CONFLICT(task_id) DO UPDATE SET state='running',generation=excluded.generation,owner=excluded.owner,lease_until_ms=excluded.lease_until_ms",&[&&id[..],&generation,&owner,&to_sql_integer(until,"lease")?])?;
            let wakeup=if let Some(w)=wake {
                tx.execute("UPDATE durable_wakeups SET lease_owner=$2,lease_until_ms=$3 WHERE task_id=$1",&[&&id[..],&owner,&to_sql_integer(until,"lease")?])?;
                Some(WakeupClaim{task_id:task,sequence:w.get::<_,i64>(0) as u64,wake_at_ms:w.get::<_,i64>(1) as u64,lease_owner:owner.into(),lease_until_ms:until})
            }else{None};
            if now()? >= until {return Ok(None);}
            tx.commit()?;
            Ok(Some(ExecutionClaim{task_id:task,owner:owner.into(),generation:generation as u64,lease_until_ms:until,wakeup}))
        })
    }
    pub(super) fn release_run(
        &self,
        claim: &ExecutionClaim,
        failed: bool,
    ) -> Result<(), DurableError> {
        let scoped = Self { pool: self.pool.clone(), execution: Some(claim.clone()) };
        self.with_client(|client| {
            let mut tx=client.transaction()?;
            scoped.guard_execution(&mut tx,claim.task_id)?;
            let id=task_id_bytes(claim.task_id);
            tx.execute("UPDATE durable_wakeups SET lease_owner=NULL,lease_until_ms=NULL WHERE task_id=$1",&[&&id[..]])?;
            scoped.guard_execution(&mut tx,claim.task_id)?;
            tx.execute("UPDATE durable_executions SET state=$2,owner=NULL,lease_until_ms=NULL WHERE task_id=$1",&[&&id[..],&if failed{"failed"}else{"suspended"}])?;
            tx.commit()?;Ok(())
        })
    }
}
