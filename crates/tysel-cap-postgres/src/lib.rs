//! Official Postgres capability.
//!
//! Trusted-path `tysel.postgres.exec` / `tysel.postgres.query` use a URL
//! resolved by the host from `TYSEL_POSTGRES_<NAME>`. Unconfigured processes
//! return `"postgres is not configured"`. Isolated workers never call this crate.

use std::error::Error;
use std::pin::pin;
use std::sync::{Arc, RwLock};

use bytes::BytesMut;
use futures_util::StreamExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_postgres::Client;
use tokio_postgres::types::{IsNull, ToSql, Type, to_sql_checked};
use tysel_engine::Value;

const MAX_SQL_BYTES: usize = 1_048_576;
const MAX_PARAMS: usize = 999;
const MAX_ROWS: usize = 10_000;
const MAX_RESULT_BYTES: usize = 1_048_576;
const MAX_CONNECTIONS: usize = 4;

struct Pool {
    url: String,
    read_only: bool,
    slots: Arc<Semaphore>,
    idle: std::sync::Mutex<Vec<Client>>,
}

static POOL: RwLock<Option<Arc<Pool>>> = RwLock::new(None);

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Replace the process-wide connection URL. `None` or a blank string leaves
/// Postgres unconfigured. Existing pooled sessions are dropped.
pub fn configure(url: Option<String>, read_only: bool) {
    let pool = url.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Arc::new(Pool {
                url: trimmed.to_owned(),
                read_only,
                slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                idle: std::sync::Mutex::new(Vec::new()),
            }))
        }
    });
    *POOL.write().expect("postgres pool lock") = pool;
}

pub async fn exec(sql: &str, params_json: &str) -> Result<f64, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    let mut checkout = checkout(true).await?;
    let refs = param_refs(&params);
    let result = checkout.client().execute(sql, &refs).await.map_err(pg_err).map(|n| n as f64);
    if result.is_err() {
        checkout.discard();
    }
    result
}

pub async fn query(sql: &str, params_json: &str) -> Result<Value, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    let mut checkout = checkout(false).await?;
    let refs = param_refs(&params);
    let result = collect_rows(checkout.client(), sql, refs).await;
    if result.is_err() {
        checkout.discard();
    }
    result
}

struct Checkout {
    client: Option<Client>,
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
}

impl Checkout {
    fn client(&self) -> &Client {
        self.client.as_ref().expect("postgres checkout")
    }

    fn discard(&mut self) {
        self.client.take();
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            if !client.is_closed() {
                self.pool.idle.lock().expect("postgres idle lock").push(client);
            }
        }
    }
}

async fn checkout(write: bool) -> Result<Checkout, String> {
    let pool =
        POOL.read().expect("postgres pool lock").clone().ok_or("postgres is not configured")?;
    if write && pool.read_only {
        return Err("postgres connection is read-only".into());
    }
    let permit = pool
        .slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "postgres pool is closed".to_string())?;
    let idle = pool.idle.lock().expect("postgres idle lock").pop();
    let client = match idle {
        Some(client) if !client.is_closed() => client,
        _ => open_client(&pool.url, pool.read_only).await?,
    };
    Ok(Checkout { client: Some(client), pool, _permit: permit })
}

async fn open_client(url: &str, read_only: bool) -> Result<Client, String> {
    let tls = postgres_native_tls::MakeTlsConnector::new(
        native_tls::TlsConnector::new().map_err(|err| err.to_string())?,
    );
    let (client, connection) = tokio_postgres::connect(url, tls).await.map_err(pg_err)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    if read_only {
        client.batch_execute("SET default_transaction_read_only = on").await.map_err(pg_err)?;
    }
    Ok(client)
}

async fn collect_rows(
    client: &Client,
    sql: &str,
    refs: Vec<&(dyn ToSql + Sync)>,
) -> Result<Value, String> {
    let stream = client.query_raw(sql, refs).await.map_err(pg_err)?;
    let mut stream = pin!(stream);
    let mut out = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = stream.next().await {
        let row = row.map_err(pg_err)?;
        if out.len() >= MAX_ROWS {
            return Err(format!("postgres query exceeded {MAX_ROWS} rows"));
        }
        let mut record = Vec::with_capacity(row.len());
        for (i, column) in row.columns().iter().enumerate() {
            record.push((column.name().to_owned(), cell(&row, i)?));
        }
        let value = Value::Record(record);
        bytes = bytes.saturating_add(value_bytes(&value));
        if bytes > MAX_RESULT_BYTES {
            return Err(format!("postgres query exceeded {MAX_RESULT_BYTES} bytes"));
        }
        out.push(value);
    }
    Ok(Value::Array(out))
}

fn check_sql(sql: &str) -> Result<&str, String> {
    if sql.trim().is_empty() {
        return Err("sql must not be empty".into());
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err("sql exceeds 1 MiB".into());
    }
    Ok(sql)
}

#[derive(Debug, Clone)]
enum Param {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
}

impl ToSql for Param {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Bool(value) => value.to_sql(ty, out),
            Self::I64(value) => encode_int(*value, ty, out),
            Self::F64(value) => encode_float(*value, ty, out),
            Self::Text(value) => value.to_sql(ty, out),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(
            *ty,
            Type::BOOL
                | Type::INT2
                | Type::INT4
                | Type::INT8
                | Type::FLOAT4
                | Type::FLOAT8
                | Type::TEXT
                | Type::VARCHAR
                | Type::BPCHAR
                | Type::NAME
                | Type::UNKNOWN
        )
    }

    to_sql_checked!();
}

fn encode_int(
    value: i64,
    ty: &Type,
    out: &mut BytesMut,
) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
    if *ty == Type::INT2 {
        i16::try_from(value)?.to_sql(ty, out)
    } else if *ty == Type::INT4 {
        i32::try_from(value)?.to_sql(ty, out)
    } else if *ty == Type::INT8 || *ty == Type::UNKNOWN {
        value.to_sql(&Type::INT8, out)
    } else {
        Err(format!("cannot bind integer to {}", ty.name()).into())
    }
}

fn encode_float(
    value: f64,
    ty: &Type,
    out: &mut BytesMut,
) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
    if *ty == Type::FLOAT4 {
        (value as f32).to_sql(ty, out)
    } else if *ty == Type::FLOAT8 || *ty == Type::UNKNOWN {
        value.to_sql(&Type::FLOAT8, out)
    } else {
        Err(format!("cannot bind float to {}", ty.name()).into())
    }
}

fn parse_params(params_json: &str) -> Result<Vec<Param>, String> {
    let raw = params_json.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid postgres params: {err}"))?;
    let serde_json::Value::Array(items) = parsed else {
        return Err("postgres params must be a JSON array".into());
    };
    if items.len() > MAX_PARAMS {
        return Err(format!("postgres params exceed {MAX_PARAMS}"));
    }
    items.iter().map(json_to_param).collect()
}

fn json_to_param(value: &serde_json::Value) -> Result<Param, String> {
    match value {
        serde_json::Value::Null => Ok(Param::Null),
        serde_json::Value::Bool(flag) => Ok(Param::Bool(*flag)),
        serde_json::Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                Ok(Param::I64(int))
            } else if let Some(float) = number.as_f64() {
                Ok(Param::F64(float))
            } else {
                Err("postgres number out of range".into())
            }
        }
        serde_json::Value::String(text) => Ok(Param::Text(text.clone())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err("postgres params must be null, bool, number, or string".into())
        }
    }
}

fn param_refs(params: &[Param]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|param| param as &(dyn ToSql + Sync)).collect()
}

fn cell(row: &tokio_postgres::Row, index: usize) -> Result<Value, String> {
    if let Ok(value) = row.try_get::<_, Option<bool>>(index) {
        return Ok(value.map_or(Value::Null, Value::Bool));
    }
    if let Ok(value) = row.try_get::<_, Option<i32>>(index) {
        return Ok(value.map_or(Value::Null, |int| Value::Number(f64::from(int))));
    }
    if let Ok(value) = row.try_get::<_, Option<i64>>(index) {
        return Ok(value.map_or(Value::Null, |int| Value::Number(int as f64)));
    }
    if let Ok(value) = row.try_get::<_, Option<f32>>(index) {
        return Ok(value.map_or(Value::Null, |float| Value::Number(f64::from(float))));
    }
    if let Ok(value) = row.try_get::<_, Option<f64>>(index) {
        return Ok(value.map_or(Value::Null, Value::Number));
    }
    if let Ok(value) = row.try_get::<_, Option<String>>(index) {
        return Ok(value.map_or(Value::Null, Value::String));
    }
    if let Ok(value) = row.try_get::<_, Option<Vec<u8>>>(index) {
        return Ok(value.map_or(Value::Null, Value::Bytes));
    }
    let ty = row.columns().get(index).map(|column| column.type_().name()).unwrap_or("unknown");
    Err(format!("unsupported postgres column type {ty}"))
}

fn value_bytes(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 8,
        Value::String(text) => text.len(),
        Value::Bytes(bytes) => bytes.len(),
        Value::Array(items) => items.iter().map(value_bytes).sum(),
        Value::Record(fields) => {
            fields.iter().map(|(key, item)| key.len().saturating_add(value_bytes(item))).sum()
        }
    }
}

fn pg_err(err: tokio_postgres::Error) -> String {
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
    fn parse_params_roundtrips_scalars() {
        let params = parse_params(r#"[null, true, 3, 1.5, "o'reilly"]"#).unwrap();
        assert!(matches!(params[0], Param::Null));
        assert!(matches!(params[1], Param::Bool(true)));
        assert!(matches!(params[2], Param::I64(3)));
        assert!(matches!(params[3], Param::F64(_)));
        assert!(matches!(&params[4], Param::Text(text) if text == "o'reilly"));
    }

    #[test]
    fn rejects_non_array_params() {
        let err = parse_params(r#"{"id":1}"#).unwrap_err();
        assert!(err.contains("JSON array"), "{err}");
    }

    #[test]
    fn json_integers_encode_as_int4() {
        let param = json_to_param(&serde_json::json!(3)).unwrap();
        let mut int4 = BytesMut::new();
        param.to_sql_checked(&Type::INT4, &mut int4).unwrap();
        assert_eq!(int4.len(), 4);
        let mut int8 = BytesMut::new();
        param.to_sql_checked(&Type::INT8, &mut int8).unwrap();
        assert_eq!(int8.len(), 8);
        let mut int2 = BytesMut::new();
        param.to_sql_checked(&Type::INT2, &mut int2).unwrap();
        assert_eq!(int2.len(), 2);
    }

    #[test]
    fn int4_rejects_out_of_range() {
        let param = Param::I64(i64::from(i32::MAX) + 1);
        let mut buf = BytesMut::new();
        assert!(param.to_sql_checked(&Type::INT4, &mut buf).is_err());
    }

    #[test]
    fn value_bytes_counts_record_payload() {
        let row = Value::Record(vec![("name".into(), Value::String("hi".into()))]);
        assert_eq!(value_bytes(&row), 6);
    }

    #[tokio::test]
    async fn unconfigured_query_errors() {
        let err = query("SELECT 1", "[]").await.unwrap_err();
        assert!(err.contains("not configured"), "{err}");
    }

    #[test]
    fn default_sslmode_prefers_tls() {
        let config: tokio_postgres::Config = "postgres://tysel@127.0.0.1/tysel".parse().unwrap();
        assert_eq!(config.get_ssl_mode(), tokio_postgres::config::SslMode::Prefer);
    }

    #[test]
    fn sslmode_query_param_is_honored() {
        let require: tokio_postgres::Config =
            "postgres://tysel@127.0.0.1/tysel?sslmode=require".parse().unwrap();
        assert_eq!(require.get_ssl_mode(), tokio_postgres::config::SslMode::Require);
        let disable: tokio_postgres::Config =
            "postgres://tysel@127.0.0.1/tysel?sslmode=disable".parse().unwrap();
        assert_eq!(disable.get_ssl_mode(), tokio_postgres::config::SslMode::Disable);
    }
}
