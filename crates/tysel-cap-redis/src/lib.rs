//! Official Redis capability.
//!
//! Trusted-path operations use a URL resolved by the host from
//! `TYSEL_REDIS_<NAME>`. The API intentionally exposes a bounded key/value
//! subset instead of arbitrary Redis commands.

use std::sync::{Arc, RwLock};

use redis::aio::ConnectionManager;
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore};
use tysel_engine::Value;

const MAX_KEY_BYTES: usize = 4 * 1024;
const MAX_VALUE_BYTES: usize = 1024 * 1024;
const MAX_KEYS: usize = 128;
const MAX_TTL_SECONDS: u64 = 31_536_000;
const MAX_IN_FLIGHT: usize = 4;

struct Config {
    url: String,
    read_only: bool,
    slots: Arc<Semaphore>,
    manager: OnceCell<ConnectionManager>,
}

static CONFIG: RwLock<Option<Arc<Config>>> = RwLock::new(None);

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Replace the process-wide Redis URL. `None` or a blank string leaves Redis
/// unconfigured. Future operations use the replacement configuration.
pub fn configure(url: Option<String>, read_only: bool) {
    let config = url.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Arc::new(Config {
            url: trimmed.to_owned(),
            read_only,
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            manager: OnceCell::new(),
        }))
    });
    *CONFIG.write().expect("redis config lock") = config;
}

pub async fn get(key: &str) -> Result<Value, String> {
    check_key(key)?;
    let (_permit, mut connection, _) = connection(false).await?;
    // GETRANGE bounds the Redis bulk response itself. Pair it with EXISTS in
    // one transaction to preserve GET's distinction between a missing key and
    // an existing empty string without a time-of-check/time-of-use race.
    let (bytes, exists): (Vec<u8>, bool) = redis::pipe()
        .atomic()
        .cmd("GETRANGE")
        .arg(key)
        .arg(0)
        .arg(MAX_VALUE_BYTES)
        .cmd("EXISTS")
        .arg(key)
        .query_async(&mut connection)
        .await
        .map_err(redis_err)?;
    match (exists, bytes) {
        (_, bytes) if bytes.len() > MAX_VALUE_BYTES => {
            Err(format!("redis value exceeded {MAX_VALUE_BYTES} bytes"))
        }
        (true, bytes) => String::from_utf8(bytes)
            .map(Value::String)
            .map_err(|_| "redis value is not valid UTF-8".into()),
        (false, _) => Ok(Value::Null),
    }
}

pub async fn set(key: &str, value: &str, ttl_seconds: Option<u64>) -> Result<Value, String> {
    check_key(key)?;
    check_value(value)?;
    if let Some(ttl) = ttl_seconds {
        check_ttl(ttl)?;
    }
    let (_permit, mut connection, _) = connection(true).await?;
    let mut command = redis::cmd("SET");
    command.arg(key).arg(value);
    if let Some(ttl) = ttl_seconds {
        command.arg("EX").arg(ttl);
    }
    command.query_async::<String>(&mut connection).await.map(|_| Value::Null).map_err(redis_err)
}

pub async fn del(keys: &[String]) -> Result<Value, String> {
    check_keys(keys)?;
    let (_permit, mut connection, _) = connection(true).await?;
    redis::cmd("DEL")
        .arg(keys)
        .query_async::<u64>(&mut connection)
        .await
        .map(|count| Value::Number(count as f64))
        .map_err(redis_err)
}

pub async fn exists(key: &str) -> Result<Value, String> {
    check_key(key)?;
    let (_permit, mut connection, _) = connection(false).await?;
    redis::cmd("EXISTS")
        .arg(key)
        .query_async::<bool>(&mut connection)
        .await
        .map(Value::Bool)
        .map_err(redis_err)
}

pub async fn expire(key: &str, ttl_seconds: u64) -> Result<Value, String> {
    check_key(key)?;
    check_ttl(ttl_seconds)?;
    let (_permit, mut connection, _) = connection(true).await?;
    redis::cmd("EXPIRE")
        .arg(key)
        .arg(ttl_seconds)
        .query_async::<bool>(&mut connection)
        .await
        .map(Value::Bool)
        .map_err(redis_err)
}

async fn connection(
    write: bool,
) -> Result<(OwnedSemaphorePermit, ConnectionManager, Arc<Config>), String> {
    let config =
        CONFIG.read().expect("redis config lock").clone().ok_or("redis is not configured")?;
    if write && config.read_only {
        return Err("redis connection is read-only".into());
    }
    let permit = config
        .slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "redis operation gate is closed".to_string())?;
    let connection = config
        .manager
        .get_or_try_init(|| async {
            let client = redis::Client::open(config.url.as_str()).map_err(redis_err)?;
            ConnectionManager::new(client).await.map_err(redis_err)
        })
        .await?
        .clone();
    Ok((permit, connection, config))
}

fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("redis key must not be empty".into());
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(format!("redis key exceeded {MAX_KEY_BYTES} bytes"));
    }
    Ok(())
}

fn check_keys(keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Err("redis del requires at least one key".into());
    }
    if keys.len() > MAX_KEYS {
        return Err(format!("redis del exceeded {MAX_KEYS} keys"));
    }
    keys.iter().try_for_each(|key| check_key(key))
}

fn check_value(value: &str) -> Result<(), String> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(format!("redis value exceeded {MAX_VALUE_BYTES} bytes"));
    }
    Ok(())
}

fn check_ttl(ttl_seconds: u64) -> Result<(), String> {
    if ttl_seconds == 0 || ttl_seconds > MAX_TTL_SECONDS {
        return Err(format!("redis TTL must be between 1 and {MAX_TTL_SECONDS} seconds"));
    }
    Ok(())
}

fn redis_err(error: redis::RedisError) -> String {
    match error.code() {
        Some(code) => format!("redis server error ({code})"),
        None if error.is_io_error() => "redis connection failed".into(),
        None => "redis operation failed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bounds() {
        assert_eq!(check_key("").unwrap_err(), "redis key must not be empty");
        assert!(check_key(&"k".repeat(MAX_KEY_BYTES + 1)).is_err());
        assert!(check_value(&"v".repeat(MAX_VALUE_BYTES + 1)).is_err());
        assert!(check_ttl(0).is_err());
        assert!(check_ttl(MAX_TTL_SECONDS + 1).is_err());
    }

    #[tokio::test]
    async fn unconfigured_and_read_only_are_denied_without_connecting() {
        configure(None, false);
        assert_eq!(get("key").await.unwrap_err(), "redis is not configured");
        configure(Some("redis://127.0.0.1:1".into()), true);
        assert_eq!(set("key", "value", None).await.unwrap_err(), "redis connection is read-only");
    }
}
