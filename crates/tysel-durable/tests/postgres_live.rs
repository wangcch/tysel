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

    let expired = task(4);
    store.append_event_json_with_wakeup_at(expired, 0, "nap".into(), "null", 40, 40).unwrap();
    let claim = store.claim_wakeup(expired, 40, "runner-a", 100).unwrap().unwrap();
    assert_eq!(claim.lease_until_ms, 140);
    assert!(store.renew_wakeup_claim(&claim, 140, 100).unwrap().is_none());

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

#[test]
fn postgres_completion_serializes_with_writers_and_prunes_only_terminal_tasks() {
    let Some(store) = store() else {
        return;
    };
    let initial = store.program_count().unwrap();
    let live = task(100);
    store.put_module(live, "export default async () => 1", 0).unwrap();
    store.append_event_json_with_wakeup_at(live, 0, "sleep".into(), "null", 0, 1).unwrap();
    assert!(matches!(
        store.complete_task(live, 1, &json!(1), 10),
        Err(DurableError::TaskSuspended { .. })
    ));
    let mut completed = Vec::new();
    for offset in 101..113 {
        let id = task(offset);
        store.put_module(id, "export default async () => 1", 0).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let writer = {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.append_event_json_at(id, 0, EventKind::Step, "once".into(), "1", 1)
            })
        };
        let finisher = {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.complete_task(id, 0, &json!(42), 10)
            })
        };
        barrier.wait();
        let written = writer.join().unwrap();
        let finished = finisher.join().unwrap();
        match (written, finished) {
            (Ok(_), Err(DurableError::HistoryConflict { .. })) => {
                store.complete_task(id, 1, &json!(42), 10).unwrap();
            }
            (Err(DurableError::TaskCompleted { .. }), Ok(true)) => {}
            other => panic!("non-serial terminal mutation: {other:?}"),
        }
        assert!(matches!(
            store.send_signal(id, "late", &json!(true), 11),
            Err(DurableError::TaskCompleted { .. })
        ));
        assert!(matches!(store.remove_program(id), Err(DurableError::TaskCompleted { .. })));
        assert_eq!(store.completion(id).unwrap().unwrap().value, json!(42));
        completed.push(id);
    }
    assert_eq!(store.program_count().unwrap(), initial + 1);
    assert_eq!(store.prune_completed(10, 100).unwrap(), 0);
    // Concurrent collectors use the same task lock as completion and event writers.
    let collectors: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.prune_completed(11, 3).unwrap())
        })
        .collect();
    let deleted: usize = collectors.into_iter().map(|thread| thread.join().unwrap()).sum();
    assert!((3..=6).contains(&deleted));
    let remaining = store.prune_completed(11, 100).unwrap();
    assert_eq!(deleted + remaining, completed.len());
    for id in completed {
        assert!(store.program(id).unwrap().is_none());
        assert!(store.completion(id).unwrap().is_none());
    }
    assert!(store.wakeup(live).unwrap().is_some());
    assert_eq!(store.load_history(live).unwrap().events.len(), 1);
    assert_eq!(store.program_count().unwrap(), initial + 1);
}

#[test]
fn postgres_v1_migration_keeps_unclassified_work_and_reopens_completion_counters() {
    let Ok(url) = std::env::var("TYSEL_POSTGRES_TEST_URL") else {
        return;
    };
    let schema = format!("tysel_migration_{}", task(300).0);
    let mut admin = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    admin.batch_execute(&format!("CREATE SCHEMA {schema}")).unwrap();
    let scoped = if url.contains("://") {
        format!(
            "{url}{}options=-csearch_path%3D{schema}",
            if url.contains('?') { "&" } else { "?" }
        )
    } else {
        format!("{url} options='-c search_path={schema}'")
    };
    {
        let store = PostgresStore::connect(&scoped).unwrap();
        store.put_program(TaskId(1), "1", 0).unwrap();
    }
    {
        let mut client = postgres::Client::connect(&scoped, postgres::NoTls).unwrap();
        client.batch_execute("DROP TABLE durable_completions; DROP TABLE durable_program_stats; UPDATE tysel_durable_metadata SET value=1 WHERE key='schema_version'").unwrap();
    }
    {
        let store = PostgresStore::connect(&scoped).unwrap();
        assert_eq!(store.log_version().unwrap(), 2);
        assert_eq!(store.program_count().unwrap(), 1);
        assert_eq!(store.prune_completed(100, 100).unwrap(), 0);
        store.complete_task(TaskId(1), 0, &json!(42), 1).unwrap();
    }
    {
        let store = PostgresStore::connect(&scoped).unwrap();
        assert_eq!(store.program_count().unwrap(), 0);
        assert_eq!(store.completion(TaskId(1)).unwrap().unwrap().value, json!(42));
        store.put_program(TaskId(2), "2", 2).unwrap();
        assert_eq!(store.program_count().unwrap(), 1);
    }
    admin.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).unwrap();
}
