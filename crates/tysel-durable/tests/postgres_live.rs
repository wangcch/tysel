use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use tysel_durable::{
    DURABLE_LOG_VERSION, DurableError, DurableProgramKind, DurableStore, EventKind, PostgresStore,
};
use tysel_task::TaskId;

fn store() -> Option<Arc<PostgresStore>> {
    let url = std::env::var("TYSEL_POSTGRES_TEST_URL").ok()?;
    Some(Arc::new(PostgresStore::connect_with_pool_size(&url, 8).expect("connect durable store")))
}

fn task(offset: u128) -> TaskId {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    TaskId((u128::from(std::process::id()) << 96) ^ nanos ^ offset)
}

#[test]
fn postgres_preserves_replay_claim_signal_and_catalog_contracts() {
    let Some(store) = store() else {
        return;
    };
    assert_eq!(store.log_version().unwrap(), DURABLE_LOG_VERSION);

    let concurrent = task(1);
    let barrier = Arc::new(Barrier::new(3));
    let mut writers = Vec::new();
    for value in [1, 2] {
        let store = store.clone();
        let barrier = barrier.clone();
        writers.push(std::thread::spawn(move || {
            barrier.wait();
            store.append_event_json_at(
                concurrent,
                0,
                EventKind::Step,
                "once".into(),
                &value.to_string(),
                10,
            )
        }));
    }
    barrier.wait();
    let results: Vec<_> = writers.into_iter().map(|writer| writer.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(DurableError::HistoryConflict { .. })))
            .count(),
        1
    );
    assert_eq!(store.load_history(concurrent).unwrap().events.len(), 1);

    let sleeping = task(2);
    store.append_event_json_with_wakeup_at(sleeping, 0, "nap".into(), "null", 20, 25).unwrap();
    store.put_module(sleeping, "export default async () => 1", 20).unwrap();
    let due = store.load_due_programs_by_kind(25, DurableProgramKind::Module).unwrap();
    assert!(due.iter().any(|program| program.task_id == sleeping));
    let claim = store.claim_wakeup(sleeping, 25, "runner-a", 100).unwrap().unwrap();
    assert!(store.claim_is_active(&claim, 25).unwrap());
    assert!(store.claim_wakeup(sleeping, 25, "runner-b", 100).unwrap().is_none());
    assert!(!store.complete_wakeup(sleeping, 0, Some("runner-b"), 25).unwrap());
    assert!(store.complete_wakeup(sleeping, 0, Some("runner-a"), 25).unwrap());

    let signaled = task(3);
    assert!(store.poll_signal(signaled, 0, "approval", 30, None).unwrap().is_none());
    let signal_id = store.send_signal(signaled, "approval", &json!({"ok": true}), 31).unwrap();
    assert!(signal_id > 0);
    let claim = store.claim_wakeup(signaled, 31, "runner-a", 100).unwrap().unwrap();
    let event = store.poll_signal(signaled, 0, "approval", 31, Some(&claim)).unwrap().unwrap();
    assert_eq!(event.kind, EventKind::Signal);
    assert_eq!(event.payload, json!({"ok": true}));
    assert!(store.wakeup(signaled).unwrap().is_none());
    assert!(store.signal_wait(signaled).unwrap().is_none());
}
