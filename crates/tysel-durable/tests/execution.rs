use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tysel_durable::{
    DurableError, DurableProgramKind, DurableStore, EventKind, PostgresStore, SqliteStore,
    admission_task_id,
};
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}
fn contract(store: Arc<dyn DurableStore>) {
    let key = format!(
        "execution-test-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    );
    let id = admission_task_id(&key).unwrap();
    let source = "export default async () => 42";
    store.admit_module(id, &key, source, "{\"n\":1}", now()).unwrap();
    store.admit_module(id, &key, source, "{ \"n\": 1 }", now()).unwrap();
    assert!(matches!(
        store.admit_module(id, &key, source, "2", now()),
        Err(DurableError::AdmissionConflict)
    ));
    assert_eq!(store.load_history(id).unwrap().events.len(), 1);
    assert!(
        store
            .load_due_programs_batch(now(), DurableProgramKind::Module, 100)
            .unwrap()
            .iter()
            .any(|p| p.task_id == id)
    );
    let first = store.claim_execution(id, "same-owner", 150).unwrap().unwrap();
    assert!(store.claim_execution(id, "same-owner", 150).unwrap().is_none());
    let stale = store.execution_store(&first);
    // An active task remains eligible after process death even without a wakeup.
    std::thread::sleep(Duration::from_millis(180));
    let second = store.claim_execution(id, "same-owner", 5_000).unwrap().unwrap();
    assert!(second.generation > first.generation);
    assert!(matches!(store.finish_execution(&first, false), Err(DurableError::ExecutionLeaseLost)));
    assert!(matches!(
        stale.append_event_json_at(id, 1, EventKind::Effect, "write".into(), "42", now()),
        Err(DurableError::ExecutionLeaseLost)
    ));
    assert!(matches!(
        stale.complete_task_before(id, 1, &json!(42), u64::MAX),
        Err(DurableError::ExecutionLeaseLost)
    ));
    let scoped = store.execution_store(&second);
    scoped.append_event_json_at(id, 1, EventKind::Effect, "write".into(), "42", now()).unwrap();
    let signal = store.send_signal_once(id, "approval", &json!(true), "decision-1", now()).unwrap();
    assert_eq!(
        store.send_signal_once(id, "approval", &json!(true), "decision-1", now()).unwrap(),
        signal
    );
    assert!(matches!(
        store.send_signal_once(id, "approval", &json!(false), "decision-1", now()),
        Err(DurableError::AdmissionConflict)
    ));
    assert!(scoped.poll_signal(id, 2, "approval", now(), None).unwrap().is_some());
    assert_eq!(
        store.send_signal_once(id, "approval", &json!(true), "decision-1", now()).unwrap(),
        signal
    );
    assert_eq!(store.load_history(id).unwrap().events.len(), 3);
    assert!(scoped.complete_task_before(id, 3, &json!(42), second.lease_until_ms).unwrap());
    store.admit_module(id, &key, source, "{\"n\":1}", now()).unwrap();
    assert_eq!(
        store.send_signal_once(id, "approval", &json!(true), "decision-1", now()).unwrap(),
        signal
    );
    assert!(store.claim_execution(id, "third", 5_000).unwrap().is_none());
    assert!(scoped.complete_task_before(id, 3, &json!(42), 0).unwrap());
    let failed_key = format!("{key}-failed");
    let failed_id = admission_task_id(&failed_key).unwrap();
    store.admit_module(failed_id, &failed_key, source, "null", now()).unwrap();
    assert!(!store.execution_failed(failed_id).unwrap());
    let claim = store.claim_execution(failed_id, "failed-owner", 5000).unwrap().unwrap();
    store.finish_execution(&claim, true).unwrap();
    assert!(store.execution_failed(failed_id).unwrap());
    store.admit_module(failed_id, &failed_key, source, "null", now()).unwrap();
    assert!(store.execution_failed(failed_id).unwrap());
}
#[test]
fn sqlite_execution_and_idempotency_contract() {
    contract(Arc::new(SqliteStore::in_memory().unwrap()));
}
#[test]
fn postgres_execution_and_idempotency_contract() {
    let Ok(url) = std::env::var("TYSEL_POSTGRES_TEST_URL") else {
        return;
    };
    contract(Arc::new(PostgresStore::connect(&url).unwrap()));
}
