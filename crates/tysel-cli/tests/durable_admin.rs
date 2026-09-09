use std::process::Command;
use tysel_durable::SqliteStore;
use tysel_task::TaskId;

#[test]
fn retained_results_and_bounded_pruning_do_not_load_application_or_delete_live_tasks() {
    let root = std::env::temp_dir().join(format!(
        "tysel-admin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("tysel.toml"), "[app]\nname = 'admin-fixture'\nentry = 'missing.js'\n[durable]\nstore = 'sqlite'\npath = './data/app.db'\n").unwrap();
    let command = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tysel"))
            .arg("-C")
            .arg(&root)
            .args(args)
            .env_remove("TYSEL_DURABLE_POSTGRES_URL")
            .env_remove("TYSEL_DURABLE_SQLITE_PATH")
            .output()
            .unwrap()
    };
    assert!(!command(&["durable", "result", "1"]).status.success());
    assert!(!root.join("data/durable-events.db").exists());
    let store = SqliteStore::open(root.join("data/durable-events.db")).unwrap();
    for id in [1, 2, 3] {
        store.put_program(TaskId(id), "1", 0).unwrap();
    }
    for id in [1, 2] {
        store.complete_task(TaskId(id), 0, &serde_json::json!({"answer":42}), 1).unwrap();
    }
    store.append_event_json_with_wakeup_at(TaskId(3), 0, "sleep".into(), "null", 0, 1).unwrap();
    let result = command(&["durable", "result", "1"]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["completion"]["value"]["answer"], 42);
    assert!(!command(&["durable", "prune", "--limit", "101"]).status.success());
    let pruned = command(&["durable", "prune", "--older-than-secs", "0", "--limit", "1"]);
    assert!(pruned.status.success(), "{}", String::from_utf8_lossy(&pruned.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&pruned.stdout).unwrap()["deleted"], 1);
    assert!(store.completion(TaskId(1)).unwrap().is_none());
    assert!(store.completion(TaskId(2)).unwrap().is_some());
    assert!(store.wakeup(TaskId(3)).unwrap().is_some());
    assert_eq!(store.program_count().unwrap(), 1);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
