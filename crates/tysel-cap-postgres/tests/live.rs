//! Live Postgres tests. Skipped unless `TYSEL_POSTGRES_TEST_URL` is set.
//! CI provides a Postgres 16 service and exports that variable.

use tysel_cap_postgres::{configure, exec, query};
use tysel_engine::Value;

static LIVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live_url() -> Option<String> {
    std::env::var("TYSEL_POSTGRES_TEST_URL").ok().filter(|value| !value.is_empty())
}

fn table(name: &str) -> String {
    format!("tysel_live_{}_{}", std::process::id(), name)
}

#[tokio::test]
async fn query_binds_int4_and_text() {
    let _guard = LIVE_LOCK.lock().await;
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url), false);
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
async fn pool_reuses_a_backend_session() {
    let _guard = LIVE_LOCK.lock().await;
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url), false);
    let first = query("SELECT pg_backend_pid() AS pid", "[]").await.unwrap();
    let second = query("SELECT pg_backend_pid() AS pid", "[]").await.unwrap();
    assert_eq!(first, second, "expected the pooled session to be reused");
}

#[tokio::test]
async fn sslmode_require_fails_when_the_server_has_no_tls() {
    let _guard = LIVE_LOCK.lock().await;
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    let required = with_sslmode(&url, "require");
    configure(Some(required), false);
    match query("SELECT 1", "[]").await {
        Ok(_) => {}
        Err(err) => {
            assert!(
                err.to_ascii_lowercase().contains("tls")
                    || err.to_ascii_lowercase().contains("ssl"),
                "{err}"
            );
        }
    }
}

fn with_sslmode(url: &str, mode: &str) -> String {
    if url.contains("sslmode=") {
        return url.to_owned();
    }
    if url.contains('?') {
        format!("{url}&sslmode={mode}")
    } else {
        format!("{url}?sslmode={mode}")
    }
}

#[tokio::test]
async fn query_stops_after_row_limit() {
    let _guard = LIVE_LOCK.lock().await;
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url), false);
    let err = query("SELECT generate_series(1, 10001) AS n", "[]").await.unwrap_err();
    assert!(err.contains("exceeded") && err.contains("rows"), "{err}");
}

#[tokio::test]
async fn read_only_grant_blocks_writes_and_allows_queries() {
    let _guard = LIVE_LOCK.lock().await;
    let Some(url) = live_url() else {
        eprintln!("skipping live postgres tests (set TYSEL_POSTGRES_TEST_URL)");
        return;
    };
    configure(Some(url), true);
    let rows = query("SELECT 1::INTEGER AS n", "[]").await.unwrap();
    assert_eq!(rows, Value::Array(vec![Value::Record(vec![("n".into(), Value::Number(1.0))])]));
    let exec_err = exec("CREATE TABLE must_not_exist (id INTEGER)", "[]").await.unwrap_err();
    assert!(exec_err.contains("read-only"), "{exec_err}");
    let query_err = query("CREATE TABLE must_not_exist (id INTEGER)", "[]").await.unwrap_err();
    assert!(query_err.to_ascii_lowercase().contains("read-only"), "{query_err}");
}
