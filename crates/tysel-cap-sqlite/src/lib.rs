//! Official SQLite capability.
//!
//! The process holds one connection. Isolated workers never call this crate;
//! they reject `tysel.sqlite` over IPC.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, InterruptHandle, params_from_iter};
use tysel_engine::Value;

const MAX_SQL_BYTES: usize = 1_048_576;
const MAX_PARAMS: usize = 999;
const MAX_ROWS: usize = 10_000;

static PATH: OnceLock<String> = OnceLock::new();
static DB: OnceLock<Sqlite> = OnceLock::new();

struct Sqlite {
    conn: Mutex<Connection>,
    interrupt: InterruptHandle,
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Pin the database path. Later calls are ignored. Empty paths are ignored so
/// tests keep the default in-memory database.
pub fn configure_path(path: impl Into<String>) {
    let path = path.into();
    if path.is_empty() {
        return;
    }
    let _ = PATH.set(path);
}

pub fn interrupt() {
    if let Some(db) = DB.get() {
        db.interrupt.interrupt();
    }
}

/// Open the process-wide connection so `interrupt` has a handle.
pub fn ensure_ready() -> Result<(), String> {
    ensure().map(|_| ())
}

pub fn exec(sql: &str, params_json: &str) -> Result<f64, String> {
    with_conn(|conn| exec_sql(conn, sql, params_json))
}

pub fn query(sql: &str, params_json: &str) -> Result<Value, String> {
    with_conn(|conn| query_sql(conn, sql, params_json))
}

/// Open a SQLite database, creating parent directories for file paths.
pub fn open(path: &str) -> Result<Connection, String> {
    if path != ":memory:" {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
        }
    }
    let conn = Connection::open(path).map_err(sql_err)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(sql_err)?;
    Ok(conn)
}

fn with_conn<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let db = ensure()?;
    let guard = db.conn.lock().map_err(|_| "sqlite lock poisoned".to_string())?;
    finish(&guard, f(&guard))
}

fn finish<T>(conn: &Connection, result: Result<T, String>) -> Result<T, String> {
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

fn ensure() -> Result<&'static Sqlite, String> {
    if let Some(db) = DB.get() {
        return Ok(db);
    }
    let path = PATH.get().map(String::as_str).filter(|path| !path.is_empty()).unwrap_or(":memory:");
    let conn = open(path)?;
    let interrupt = conn.get_interrupt_handle();
    let _ = DB.set(Sqlite { conn: Mutex::new(conn), interrupt });
    DB.get().ok_or_else(|| "sqlite connection missing".into())
}

fn exec_sql(conn: &Connection, sql: &str, params_json: &str) -> Result<f64, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    if params.is_empty() {
        conn.execute_batch(sql).map_err(sql_err)?;
    } else {
        let mut stmt = conn.prepare(sql).map_err(sql_err)?;
        stmt.execute(params_from_iter(params.iter())).map_err(sql_err)?;
    }
    Ok(conn.changes() as f64)
}

fn query_sql(conn: &Connection, sql: &str, params_json: &str) -> Result<Value, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    let mut stmt = conn.prepare(sql).map_err(sql_err)?;
    let names: Vec<String> = stmt.column_names().iter().map(|name| (*name).to_owned()).collect();
    let mut rows = stmt.query(params_from_iter(params.iter())).map_err(sql_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(sql_err)? {
        let mut record = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            record.push((name.clone(), sql_to_value(row.get_ref(i).map_err(sql_err)?)?));
        }
        out.push(Value::Record(record));
        if out.len() > MAX_ROWS {
            return Err(format!("sqlite query exceeded {MAX_ROWS} rows"));
        }
    }
    Ok(Value::Array(out))
}

fn check_sql(sql: &str) -> Result<&str, String> {
    if sql.is_empty() {
        return Err("sql must not be empty".into());
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err("sql exceeds 1 MiB".into());
    }
    Ok(sql)
}

fn parse_params(params_json: &str) -> Result<Vec<SqlValue>, String> {
    let raw = params_json.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid sqlite params: {err}"))?;
    let serde_json::Value::Array(items) = parsed else {
        return Err("sqlite params must be a JSON array".into());
    };
    if items.len() > MAX_PARAMS {
        return Err(format!("sqlite params exceed {MAX_PARAMS}"));
    }
    items.iter().map(json_to_sql).collect()
}

fn json_to_sql(value: &serde_json::Value) -> Result<SqlValue, String> {
    match value {
        serde_json::Value::Null => Ok(SqlValue::Null),
        serde_json::Value::Bool(flag) => Ok(SqlValue::Integer(i64::from(*flag))),
        serde_json::Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                Ok(SqlValue::Integer(int))
            } else if let Some(float) = number.as_f64() {
                Ok(SqlValue::Real(float))
            } else {
                Err("sqlite number out of range".into())
            }
        }
        serde_json::Value::String(text) => Ok(SqlValue::Text(text.clone())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err("sqlite params must be null, bool, number, or string".into())
        }
    }
}

fn sql_to_value(value: ValueRef<'_>) -> Result<Value, String> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(int) => Ok(Value::Number(int as f64)),
        ValueRef::Real(float) => Ok(Value::Number(float)),
        ValueRef::Text(bytes) => Ok(Value::String(String::from_utf8_lossy(bytes).into_owned())),
        ValueRef::Blob(bytes) => Ok(Value::Bytes(bytes.to_vec())),
    }
}

fn sql_err(err: rusqlite::Error) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn query_roundtrips_bound_text_with_quotes() {
        let table = format!("t_{}", std::process::id());
        exec(&format!("CREATE TABLE {table} (id INTEGER, name TEXT)"), "[]").unwrap();
        let changes =
            exec(&format!("INSERT INTO {table} (id, name) VALUES (?, ?)"), r#"[1, "o'reilly"]"#)
                .unwrap();
        assert_eq!(changes, 1.0);
        let rows =
            query(&format!("SELECT id, name FROM {table} WHERE name = ?"), r#"["o'reilly"]"#)
                .unwrap();
        assert_eq!(
            rows,
            Value::Array(vec![Value::Record(vec![
                ("id".into(), Value::Number(1.0)),
                ("name".into(), Value::String("o'reilly".into())),
            ])])
        );
    }

    #[test]
    fn rejects_non_array_params() {
        let err = exec("SELECT 1", r#"{"id":1}"#).unwrap_err();
        assert!(err.contains("JSON array"), "{err}");
    }

    #[test]
    fn error_rolls_back_an_open_transaction() {
        let conn = open(":memory:").unwrap();
        exec_sql(&conn, "CREATE TABLE t (id INTEGER)", "[]").unwrap();
        exec_sql(&conn, "BEGIN", "[]").unwrap();
        exec_sql(&conn, "INSERT INTO t (id) VALUES (1)", "[]").unwrap();
        finish(&conn, exec_sql(&conn, "NOT SQL", "[]")).unwrap_err();
        let rows = query_sql(&conn, "SELECT COUNT(*) AS n FROM t", "[]").unwrap();
        assert_eq!(rows, Value::Array(vec![Value::Record(vec![("n".into(), Value::Number(0.0))])]));
    }

    #[test]
    fn interrupt_aborts_a_long_query_and_keeps_the_connection() {
        ensure_ready().unwrap();
        let worker = std::thread::spawn(|| {
            query(
                "WITH RECURSIVE t(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM t WHERE x < 200000000)
                 SELECT COUNT(*) AS n FROM t",
                "[]",
            )
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        interrupt();
        let err = worker.join().expect("join").expect_err("interrupted");
        assert!(err.to_ascii_lowercase().contains("interrupt"), "unexpected error: {err}");
        let rows = query("SELECT 1 AS n", "[]").unwrap();
        assert_eq!(rows, Value::Array(vec![Value::Record(vec![("n".into(), Value::Number(1.0))])]));
    }

    #[test]
    fn open_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "tysel-sqlite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let path = dir.join("nested").join("t.db");
        let conn = open(path.to_str().expect("utf-8 path")).expect("open");
        drop(conn);
        assert!(path.is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
