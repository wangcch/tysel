use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value as JsonValue, json};
use tokio::task::JoinHandle;
use tysel_durable::{DurableError, DurableStore, POSTGRES_URL_ENV, PostgresStore, SqliteStore};
use tysel_engine::{IsolateConfig, Value};
use tysel_engine_qjs::{
    DurableControl, configure_durable_control, encode_durable_export, inspect_durable_exports,
};
use tysel_task::TaskId;

use crate::{
    DispatchError, DurableDispatcher, DurablePoller, DurableProgramCatalog, DurableRun,
    DurableRunStatus, PollerError, PollerShutdown, ProgramRegistryError,
};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const POLL_BATCH: usize = 32;
const SQLITE_PATH_ENV: &str = "TYSEL_DURABLE_SQLITE_PATH";

pub struct DurablePlane {
    dispatcher: Arc<DurableDispatcher>,
    catalog: DurableProgramCatalog,
    source: RwLock<Arc<str>>,
    config: IsolateConfig,
    shutdown: PollerShutdown,
    join: Mutex<Option<JoinHandle<Result<(), PollerError>>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DurablePlaneError {
    #[error(transparent)]
    Store(#[from] DurableError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error(transparent)]
    Poller(#[from] PollerError),
    #[error(transparent)]
    Registry(#[from] ProgramRegistryError),
    #[error(transparent)]
    Engine(#[from] tysel_engine::EngineError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("durable control lock is poisoned")]
    Poisoned,
}

impl DurablePlane {
    pub fn open_store(
        sqlite_capability_path: &str,
        root: Option<&Path>,
    ) -> Result<Option<Arc<dyn DurableStore>>, DurablePlaneError> {
        if let Ok(url) = std::env::var(POSTGRES_URL_ENV)
            && !url.trim().is_empty()
        {
            return Ok(Some(Arc::new(PostgresStore::connect_from_env()?)));
        }
        if let Ok(path) = std::env::var(SQLITE_PATH_ENV) {
            let path = path.trim();
            if !path.is_empty() {
                let resolved = resolve_path(path, root);
                if let Some(parent) = Path::new(&resolved).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                return Ok(Some(Arc::new(SqliteStore::open(resolved)?)));
            }
        }
        let cap = sqlite_capability_path.trim();
        if cap.is_empty() || cap == ":memory:" {
            return Ok(None);
        }
        let cap_path = resolve_path(cap, root);
        let dir = Path::new(&cap_path).parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        Ok(Some(Arc::new(SqliteStore::open(dir.join("durable-events.db"))?)))
    }

    pub fn event_log_path(sqlite_capability_path: &str, root: Option<&Path>) -> Option<PathBuf> {
        if std::env::var(POSTGRES_URL_ENV).ok().is_some_and(|url| !url.trim().is_empty()) {
            return None;
        }
        if let Ok(path) = std::env::var(SQLITE_PATH_ENV) {
            let path = path.trim();
            if !path.is_empty() {
                return Some(resolve_path(path, root));
            }
        }
        let cap = sqlite_capability_path.trim();
        if cap.is_empty() || cap == ":memory:" {
            return None;
        }
        let cap_path = resolve_path(cap, root);
        let dir = Path::new(&cap_path).parent().unwrap_or(Path::new("."));
        Some(dir.join("durable-events.db"))
    }

    pub fn requested(
        sqlite_capability_path: &str,
        root: Option<&Path>,
        source: &str,
        config: IsolateConfig,
    ) -> Result<bool, DurablePlaneError> {
        if Self::has_durable_exports(source, config)? {
            return Ok(true);
        }
        if std::env::var(POSTGRES_URL_ENV).ok().is_some_and(|url| !url.trim().is_empty()) {
            return Ok(true);
        }
        if std::env::var(SQLITE_PATH_ENV).ok().is_some_and(|path| !path.trim().is_empty()) {
            return Ok(true);
        }
        Ok(Self::event_log_path(sqlite_capability_path, root).is_some_and(|path| path.exists()))
    }

    pub fn start(
        store: Arc<dyn DurableStore>,
        source: String,
        config: IsolateConfig,
        owner: impl Into<String>,
    ) -> Result<Arc<Self>, DurablePlaneError> {
        let lease_duration_ms = config.request_timeout_ms.saturating_add(5_000).max(1_000);
        let dispatcher =
            Arc::new(DurableDispatcher::new(store.clone(), owner, lease_duration_ms, config)?);
        let catalog = DurableProgramCatalog::new(store);
        let poller =
            DurablePoller::new_persistent_modules(dispatcher.clone(), POLL_INTERVAL, POLL_BATCH)?;
        let shutdown = PollerShutdown::default();
        let join = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { poller.run(shutdown, |_| {}).await }
        });
        let plane = Arc::new(Self {
            dispatcher,
            catalog,
            source: RwLock::new(Arc::from(source)),
            config,
            shutdown,
            join: Mutex::new(Some(join)),
        });
        plane.install_hooks()?;
        Ok(plane)
    }

    pub fn replace_source(&self, source: String) -> Result<(), DurablePlaneError> {
        *self.source.write().map_err(|_| DurablePlaneError::Poisoned)? = Arc::from(source);
        Ok(())
    }

    pub fn has_durable_exports(
        source: &str,
        config: IsolateConfig,
    ) -> Result<bool, DurablePlaneError> {
        Ok(!inspect_durable_exports(source, config)?.is_empty())
    }

    pub fn should_start(
        store: &dyn DurableStore,
        source: &str,
        config: IsolateConfig,
    ) -> Result<bool, DurablePlaneError> {
        Ok(Self::has_durable_exports(source, config)? || store.program_count()? > 0)
    }

    fn install_hooks(self: &Arc<Self>) -> Result<(), DurablePlaneError> {
        let plane = self.clone();
        configure_durable_control(Some(Arc::new(DurableControl {
            start: Box::new(move |name, input| plane.start_named(name, input)),
            send_signal: {
                let plane = self.clone();
                Box::new(move |task_id, name, payload| plane.send_signal(task_id, name, payload))
            },
        })));
        Ok(())
    }

    pub fn start_named(&self, name: &str, input_json: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err("durable export name must be 1..=128 bytes".into());
        }
        let source =
            { self.source.read().map_err(|_| "durable source lock poisoned".to_string())?.clone() };
        let available =
            inspect_durable_exports(source.as_ref(), self.config).map_err(|err| err.to_string())?;
        if !available.iter().any(|export| export == name) {
            return Err(format!("durable export {name} is not registered"));
        }
        let wrapped = encode_durable_export(name, source.as_ref());
        let task_id = next_task_id();
        self.catalog.register_module(task_id, wrapped.clone()).map_err(|err| err.to_string())?;
        encode_run(self.dispatcher.start_module(task_id, &wrapped, input_json))
    }

    pub fn send_signal(&self, task_id: &str, name: &str, payload_json: &str) -> Result<(), String> {
        let task_id = parse_task_id(task_id)?;
        let payload: JsonValue = serde_json::from_str(payload_json)
            .map_err(|err| format!("durable signal payload must be JSON: {err}"))?;
        let now_ms = unix_time_ms().map_err(|err| err.to_string())?;
        self.dispatcher
            .store()
            .send_signal(task_id, name, &payload, now_ms)
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    pub async fn shutdown(&self) -> Result<(), DurablePlaneError> {
        configure_durable_control(None);
        self.shutdown.cancel();
        let join = self.join.lock().map_err(|_| DurablePlaneError::Poisoned)?.take();
        if let Some(join) = join {
            join.await.map_err(PollerError::Join)??;
        }
        Ok(())
    }
}

fn encode_run(run: DurableRun) -> Result<String, String> {
    let body = match run.result {
        Ok(DurableRunStatus::Suspended) => {
            json!({ "taskId": run.task_id.to_string(), "status": "suspended" })
        }
        Ok(DurableRunStatus::Completed(value)) => json!({
            "taskId": run.task_id.to_string(),
            "status": "completed",
            "value": engine_value_to_json(&value),
        }),
        Err(error) => return Err(error.to_string()),
    };
    serde_json::to_string(&body).map_err(|err| err.to_string())
}

fn engine_value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Number(value) => {
            serde_json::Number::from_f64(*value).map(JsonValue::Number).unwrap_or(JsonValue::Null)
        }
        Value::String(value) => JsonValue::String(value.clone()),
        Value::Bytes(value) => {
            JsonValue::Array(value.iter().copied().map(JsonValue::from).collect())
        }
        Value::Array(items) => JsonValue::Array(items.iter().map(engine_value_to_json).collect()),
        Value::Record(fields) => {
            let mut map = serde_json::Map::new();
            for (key, value) in fields {
                map.insert(key.clone(), engine_value_to_json(value));
            }
            JsonValue::Object(map)
        }
    }
}

fn next_task_id() -> TaskId {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    TaskId((nanos << 16) | u128::from(n) | (u128::from(std::process::id()) << 96))
}

fn parse_task_id(raw: &str) -> Result<TaskId, String> {
    u128::from_str_radix(raw.trim(), 16)
        .map(TaskId)
        .map_err(|_| "durable task id is invalid".into())
}

fn unix_time_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_millis();
    u64::try_from(millis).map_err(|_| "system time is too large".into())
}

fn resolve_path(path: &str, root: Option<&Path>) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(root) = root {
        root.join(path)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn named_export_survives_store_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "tysel-durable-plane-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("durable-events.db");
        let source = r#"
            export default {
              durable: {
                async agent(ctx, input) {
                  const approval = await ctx.waitForSignal("approval");
                  return { input, approval };
                }
              }
            };
        "#;
        let config = IsolateConfig {
            request_timeout_ms: 500,
            cpu_ms_per_turn: 50,
            memory_limit_bytes: 8 * 1024 * 1024,
        };
        let plane = DurablePlane::start(
            Arc::new(SqliteStore::open(&path).unwrap()),
            source.into(),
            config,
            "plane-a",
        )
        .unwrap();
        let started: JsonValue =
            serde_json::from_str(&plane.start_named("agent", r#"{"n":1}"#).unwrap()).unwrap();
        assert_eq!(started["status"], "suspended");
        let task_id = started["taskId"].as_str().unwrap().to_owned();
        plane.shutdown().await.unwrap();

        let store = Arc::new(SqliteStore::open(&path).unwrap());
        let plane = DurablePlane::start(store.clone(), source.into(), config, "plane-b").unwrap();
        plane.send_signal(&task_id, "approval", r#"{"ok":true}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        plane.shutdown().await.unwrap();
        let id = parse_task_id(&task_id).unwrap();
        assert!(store.wakeup(id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
