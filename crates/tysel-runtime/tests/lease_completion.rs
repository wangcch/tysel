use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tysel_durable::SqliteStore;
use tysel_engine::{EngineError, IsolateConfig};
use tysel_engine_qjs::{DurableSession, eval_durable};
use tysel_task::TaskId;
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}
#[test]
fn completion_rejects_lease_expiry_while_waiting_for_sqlite_writer() {
    let path = std::env::temp_dir().join(format!("tysel-review-lease-{}.db", std::process::id()));
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let id = TaskId(701);
    let script = "(async () => { await tysel.durable.sleep(1); return 42; })()";
    store.put_program(id, script, now()).unwrap();
    let config = IsolateConfig::default();
    assert!(matches!(
        eval_durable(script, config, DurableSession::new(store.clone(), id).unwrap()),
        Err(EngineError::Suspended)
    ));
    std::thread::sleep(Duration::from_millis(5));
    let claim = store.claim_wakeup(id, now(), "review", 500).unwrap().unwrap();
    let until = claim.lease_until_ms;
    let session = DurableSession::from_claim(store.clone(), claim).unwrap();
    eval_durable(script, config, session.clone()).unwrap();
    let completion = session.completion_handle().unwrap();
    let blocker = rusqlite::Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = now();
    assert!(started < until);
    let thread = std::thread::spawn(move || completion.complete(&serde_json::json!(42)));
    std::thread::sleep(Duration::from_millis(800));
    assert!(!thread.is_finished(), "completion must wait for the held writer lock");
    blocker.execute_batch("ROLLBACK").unwrap();
    let result = thread.join().unwrap();
    let persisted = store.completion(id).unwrap();
    println!(
        "started_before_expiry_ms={} finished_after_expiry_ms={} result={result:?} persisted={}",
        until - started,
        now() - until,
        persisted.is_some()
    );
    assert!(matches!(result, Err(tysel_durable::DurableError::TaskSuspended { .. })));
    assert!(persisted.is_none());
    assert_eq!(store.program_count().unwrap(), 1);
    drop(blocker);
    drop(session);
    drop(store);
    std::fs::remove_file(path).unwrap();
}
