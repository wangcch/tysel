//! Live Postgres tests. Skipped unless `TYSEL_POSTGRES_TEST_URL` is set.
//! CI provides a Postgres 16 service and exports that variable.

use tysel_cap_postgres::{configure, exec, query};
use tysel_engine::Value;

fn live_url() -> Option<String> {
    std::env::var("TYSEL_POSTGRES_TEST_URL").ok().filter(|value| !value.is_empty())
}

fn table(name: &str) -> String {
    format!("tysel_live_{}_{}", std::process::id(), name)
}

#[tokio::test]
async fn query_binds_int4_and_text() {
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url));
    let table = table("int4");
    exec(&format!("DROP TABLE IF EXISTS {table}"), "[]").await.unwrap();
    exec(&format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"), "[]")
        .await
        .unwrap();
    let changes =
        exec(&format!("INSERT INTO {table} (id, name) VALUES ($1, $2)"), r#"[3, "o'reilly"]"#)
            .await
            .unwrap();
    assert_eq!(changes, 1.0);
    let rows = query(&format!("SELECT id, name FROM {table} WHERE id = $1"), "[3]").await.unwrap();
    assert_eq!(
        rows,
        Value::Array(vec![Value::Record(vec![
            ("id".into(), Value::Number(3.0)),
            ("name".into(), Value::String("o'reilly".into())),
        ])])
    );
    exec(&format!("DROP TABLE {table}"), "[]").await.unwrap();
}

#[tokio::test]
async fn query_stops_after_row_limit() {
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url));
    let err = query("SELECT generate_series(1, 10001) AS n", "[]").await.unwrap_err();
    assert!(err.contains("exceeded") && err.contains("rows"), "{err}");
}
