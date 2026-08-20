//! Official Postgres capability.
//!
//! Trusted-path `tysel.postgres.exec` / `tysel.postgres.query` use a URL
//! resolved by the host from `TYSEL_POSTGRES_<NAME>`. Unconfigured processes
//! return `"postgres is not configured"`. Isolated workers never call this crate.

use std::error::Error;
use std::pin::pin;
use std::sync::RwLock;

use bytes::BytesMut;
use futures_util::StreamExt;
use tokio_postgres::NoTls;
use tokio_postgres::types::{IsNull, ToSql, Type, to_sql_checked};
use tysel_engine::Value;

const MAX_SQL_BYTES: usize = 1_048_576;
const MAX_PARAMS: usize = 999;
const MAX_ROWS: usize = 10_000;
const MAX_RESULT_BYTES: usize = 1_048_576;

static URL: RwLock<Option<String>> = RwLock::new(None);

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Replace the process-wide connection URL. `None` or a blank string leaves
/// Postgres unconfigured.
pub fn configure(url: Option<String>) {
    let url = url.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    });
    *URL.write().expect("postgres url lock") = url;
}

pub async fn exec(sql: &str, params_json: &str) -> Result<f64, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    let client = connect().await?;
    let refs = param_refs(&params);
    let n = client.execute(sql, &refs).await.map_err(pg_err)?;
    Ok(n as f64)
}

pub async fn query(sql: &str, params_json: &str) -> Result<Value, String> {
    let sql = check_sql(sql)?;
    let params = parse_params(params_json)?;
    let client = connect().await?;
    let refs = param_refs(&params);
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

async fn connect() -> Result<tokio_postgres::Client, String> {
    let url = URL.read().expect("postgres url lock").clone().ok_or("postgres is not configured")?;
    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.map_err(pg_err)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
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
}
