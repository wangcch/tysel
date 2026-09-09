use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value as JsonValue, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tysel_durable::{DurableError, DurableStore, POSTGRES_URL_ENV, PostgresStore, SqliteStore};
use tysel_engine::{IsolateConfig, Value};
use tysel_engine_qjs::{
    DurableControl, clear_durable_control_if_current, configure_durable_control,
    encode_durable_export, inspect_durable_exports,
};
use tysel_task::TaskId;

use crate::{
    DispatchError, DurableDispatcher, DurablePoller, DurableRun, DurableRunStatus, PollerError,
    PollerHealth, PollerShutdown, ProgramRegistryError,
};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const POLL_BATCH: usize = 32;
const SQLITE_PATH_ENV: &str = "TYSEL_DURABLE_SQLITE_PATH";

pub struct DurablePlane {
    dispatcher: Arc<DurableDispatcher>,
    source: RwLock<Arc<RegisteredSource>>,
    config: IsolateConfig,
    shutdown: PollerShutdown,
    wakeup: Arc<tokio::sync::Notify>,
    health: watch::Receiver<PollerHealth>,
    join: Mutex<Option<JoinHandle<Result<(), PollerError>>>>,
    hooks: Mutex<std::sync::Weak<DurableControl>>,
}

// Publish source and its inspected exports together so reloads cannot mix generations.
struct RegisteredSource {
    text: String,
    exports: Vec<String>,
}

impl RegisteredSource {
    fn new(text: String, config: IsolateConfig) -> Result<Self, DurablePlaneError> {
        let exports = inspect_durable_exports(&text, config)?;
        Ok(Self { text, exports })
    }
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
    #[error("durable scheduler is unavailable; inspect scheduler health")]
    Unavailable,
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

    pub fn requested_with_metadata(
        sqlite_capability_path: &str,
        root: Option<&Path>,
        has_durable_exports: bool,
    ) -> bool {
        has_durable_exports
            || std::env::var(POSTGRES_URL_ENV).ok().is_some_and(|url| !url.trim().is_empty())
            || std::env::var(SQLITE_PATH_ENV).ok().is_some_and(|path| !path.trim().is_empty())
            || Self::event_log_path(sqlite_capability_path, root).is_some_and(|path| path.exists())
    }

    pub fn start(
        store: Arc<dyn DurableStore>,
        source: String,
        config: IsolateConfig,
        owner: impl Into<String>,
    ) -> Result<Arc<Self>, DurablePlaneError> {
        let source = Arc::new(RegisteredSource::new(source, config)?);
        let lease_duration_ms = config.request_timeout_ms.saturating_add(5_000).max(1_000);
        let dispatcher =
            Arc::new(DurableDispatcher::new(store.clone(), owner, lease_duration_ms, config)?);
        let poller =
            DurablePoller::new_persistent_modules(dispatcher.clone(), POLL_INTERVAL, POLL_BATCH)?;
        let wakeup = poller.wakeup();
        let shutdown = PollerShutdown::default();
        let (health_tx, health) = watch::channel(PollerHealth::Healthy);
        let join = tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                poller
                    .run_supervised(
                        shutdown,
                        |run| {
                            let state = match run.result {
                                Ok(DurableRunStatus::Completed(_)) => "completed",
                                Ok(DurableRunStatus::Suspended) => "suspended",
                                Err(_) => "task_failed",
                            };
                            tysel_observability::log_durable(state, Some(run.task_id.0), 0);
                        },
                        |health| {
                            let (state, attempt) = match health {
                                PollerHealth::Healthy => ("healthy", 0),
                                PollerHealth::Recovering { attempt } => ("recovering", attempt),
                                PollerHealth::Failed => ("failed", 0),
                            };
                            tysel_observability::log_durable(state, None, attempt);
                            health_tx.send_replace(health);
                        },
                    )
                    .await
            }
        });
        let plane = Arc::new(Self {
            dispatcher,
            source: RwLock::new(source),
            config,
            shutdown,
            wakeup,
            health,
            join: Mutex::new(Some(join)),
            hooks: Mutex::new(std::sync::Weak::new()),
        });
        plane.install_hooks()?;
        Ok(plane)
    }

    pub fn replace_source(&self, source: String) -> Result<(), DurablePlaneError> {
        let source = Arc::new(RegisteredSource::new(source, self.config)?);
        *self.source.write().map_err(|_| DurablePlaneError::Poisoned)? = source;
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

    pub fn should_start_with_metadata(
        store: &dyn DurableStore,
        has_durable_exports: bool,
    ) -> Result<bool, DurablePlaneError> {
        Ok(has_durable_exports || store.program_count()? > 0)
    }

    fn install_hooks(self: &Arc<Self>) -> Result<(), DurablePlaneError> {
        let plane = self.clone();
        let hooks = Arc::new(DurableControl {
            start: Box::new(move |name, input, key| plane.start_named_with_key(name, input, key)),
            send_signal: {
                let plane = self.clone();
                Box::new(move |task_id, name, payload, key| {
                    plane.send_signal_with_key(task_id, name, payload, key)
                })
            },
        });
        *self.hooks.lock().map_err(|_| DurablePlaneError::Poisoned)? = Arc::downgrade(&hooks);
        configure_durable_control(Some(hooks));
        Ok(())
    }

    pub fn start_named(&self, name: &str, input_json: &str) -> Result<String, String> {
        self.start_named_with_key(name, input_json, None)
    }

    pub fn start_named_with_key(
        &self,
        name: &str,
        input_json: &str,
        key: Option<&str>,
    ) -> Result<String, String> {
        if self.health() != PollerHealth::Healthy {
            return Err(DurablePlaneError::Unavailable.to_string());
        }
        let name = name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err("durable export name must be 1..=128 bytes".into());
        }
        let source =
            { self.source.read().map_err(|_| "durable source lock poisoned".to_string())?.clone() };
        if !source.exports.iter().any(|export| export == name) {
            return Err(format!("durable export {name} is not registered"));
        }
        let wrapped = encode_durable_export(name, &source.text);
        let slot = self.dispatcher.reserve().map_err(|error| error.to_string())?;
        let generated = next_task_id().to_string();
        let key = key.unwrap_or(&generated);
        let task_id = tysel_durable::admission_task_id(key).map_err(|error| error.to_string())?;
        self.dispatcher
            .store()
            .admit_module(
                task_id,
                key,
                &wrapped,
                input_json,
                unix_time_ms().map_err(|e| e.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        match self
            .dispatcher
            .start_admitted(task_id, &wrapped, slot)
            .map_err(|error| error.to_string())?
        {
            Some(run) => encode_run(run),
            None => Ok(json!({"status":"accepted","taskId":task_id.to_string()}).to_string()),
        }
    }

    pub fn send_signal(&self, task_id: &str, name: &str, payload_json: &str) -> Result<(), String> {
        self.send_signal_with_key(task_id, name, payload_json, None)
    }

    pub fn send_signal_with_key(
        &self,
        task_id: &str,
        name: &str,
        payload_json: &str,
        key: Option<&str>,
    ) -> Result<(), String> {
        let task_id = parse_task_id(task_id)?;
        let payload: JsonValue = serde_json::from_str(payload_json)
            .map_err(|err| format!("durable signal payload must be JSON: {err}"))?;
        let now_ms = unix_time_ms().map_err(|err| err.to_string())?;
        let store = self.dispatcher.store();
        match key {
            Some(key) => store.send_signal_once(task_id, name, &payload, key, now_ms),
            None => store.send_signal(task_id, name, &payload, now_ms),
        }
        .map_err(|err| err.to_string())?;
        // Wake only after commit; a lost hint is repaired by the periodic scan.
        self.wakeup.notify_one();
        Ok(())
    }

    pub fn health(&self) -> PollerHealth {
        *self.health.borrow()
    }

    pub async fn failed(&self) -> DurablePlaneError {
        let mut health = self.health.clone();
        loop {
            if *health.borrow_and_update() == PollerHealth::Failed {
                return DurablePlaneError::Unavailable;
            }
            if health.changed().await.is_err() {
                return DurablePlaneError::Unavailable;
            }
        }
    }

    pub fn stop_claiming(&self) {
        self.shutdown.cancel();
    }

    pub async fn shutdown(&self) -> Result<(), DurablePlaneError> {
        clear_durable_control_if_current(
            &*self.hooks.lock().map_err(|_| DurablePlaneError::Poisoned)?,
        );
        self.shutdown.cancel();
        let join = self.join.lock().map_err(|_| DurablePlaneError::Poisoned)?.take();
        if let Some(join) = join {
            join.await.map_err(PollerError::Join)??;
        }
        if self.dispatcher.has_pending_completions() {
            return Err(DurablePlaneError::Unavailable);
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
    async fn source_replacement_updates_exports_atomically_and_preserves_replay() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let config = IsolateConfig { cpu_ms_per_turn: 500, ..Default::default() };
        let plane = DurablePlane::start(
            store.clone(),
            "export default {durable:{async old(ctx){await ctx.waitForSignal('go');return 1}}};"
                .into(),
            config,
            "source-cache",
        )
        .unwrap();
        let started: JsonValue =
            serde_json::from_str(&plane.start_named("old", "null").unwrap()).unwrap();
        let id = parse_task_id(started["taskId"].as_str().unwrap()).unwrap();
        assert!(plane.replace_source("export default {".into()).is_err());
        assert!(plane.start_named("old", "null").is_ok());
        plane.replace_source("export default {durable:{async new(){return 2}}};".into()).unwrap();
        assert!(plane.start_named("old", "null").unwrap_err().contains("not registered"));
        let new: JsonValue =
            serde_json::from_str(&plane.start_named("new", "null").unwrap()).unwrap();
        assert_eq!(new["value"].as_f64(), Some(2.0));
        plane.send_signal(&id.to_string(), "go", "true").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while store.completion(id).unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.completion(id).unwrap().unwrap().value.as_f64(), Some(1.0));
        plane.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn task_boundary_error_does_not_stop_scheduler_or_accept_failed_retry() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let plane = DurablePlane::start(store.clone(), r#"
          export default {durable: {
            async bad(ctx) {await ctx.sleep(1);return await ctx.effect('x'.repeat(257),async()=>42)},
            async good() {return 42}
          }};
        "#.into(), IsolateConfig {request_timeout_ms:1000,..Default::default()}, "boundary-test").unwrap();
        let first: JsonValue = serde_json::from_str(
            &plane.start_named_with_key("bad", "null", Some("bad-key")).unwrap(),
        )
        .unwrap();
        let id = parse_task_id(first["taskId"].as_str().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !store.execution_failed(id).unwrap() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(plane.health(), PollerHealth::Healthy);
        let error = plane.start_named_with_key("bad", "null", Some("bad-key")).unwrap_err();
        assert!(error.contains("already failed"), "{error}");
        let good: JsonValue = serde_json::from_str(
            &plane.start_named_with_key("good", "null", Some("good-key")).unwrap(),
        )
        .unwrap();
        assert_eq!(good["value"].as_f64(), Some(42.0));
        plane.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn retry_sleep_then_rejected_approval_replays_to_completion() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let plane=DurablePlane::start(store.clone(),r#"
          export default {durable:{async job(ctx) {
            await ctx.retry({maxAttempts:2,delay:1},async attempt=>{if(attempt===1)throw new Error('retry');return true;});
            return await ctx.waitForSignal('approval');
          }}};
        "#.into(),IsolateConfig{request_timeout_ms:1_000,cpu_ms_per_turn:500,..Default::default()},"retry-approval").unwrap();
        let result: JsonValue = serde_json::from_str(
            &plane.start_named_with_key("job", "null", Some("retry-approval")).unwrap(),
        )
        .unwrap();
        let id = parse_task_id(result["taskId"].as_str().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while store.signal_wait(id).unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        plane.send_signal_with_key(&id.to_string(), "approval", "false", Some("reject-1")).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while store.completion(id).unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.completion(id).unwrap().unwrap().value, json!(false));
        plane.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn scheduler_recovers_after_store_error_without_restart() {
        let path = std::env::temp_dir().join(format!(
            "tysel-poller-recovery-{}-{}.db",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("ALTER TABLE durable_programs RENAME TO temporarily_unavailable")
            .unwrap();
        let plane = DurablePlane::start(store.clone(), r#"
            export default {durable:{async work(ctx) { await ctx.sleep(1); return await ctx.step('resumed', () => 42); }}};
        "#.into(), IsolateConfig { cpu_ms_per_turn: 500, request_timeout_ms: 1_000, ..Default::default() }, "recovery").unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !matches!(plane.health(), PollerHealth::Recovering { .. }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(plane.start_named("work", "null").is_err());
        connection
            .execute_batch("ALTER TABLE temporarily_unavailable RENAME TO durable_programs")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while plane.health() != PollerHealth::Healthy {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let started: JsonValue =
            serde_json::from_str(&plane.start_named("work", "null").unwrap()).unwrap();
        let id = parse_task_id(started["taskId"].as_str().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while store.completion(id).unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.completion(id).unwrap().unwrap().value.as_f64(), Some(42.0));
        assert_eq!(store.program_count().unwrap(), 0);
        plane.shutdown().await.unwrap();
        drop(connection);
        drop(plane);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn completion_write_failure_is_retried_without_replaying_handler() {
        let path = std::env::temp_dir().join(format!(
            "tysel-finalize-{}-{}.db",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_completion BEFORE INSERT ON durable_completions BEGIN SELECT * FROM temporarily_missing; END").unwrap();
        let plane = DurablePlane::start(
            store.clone(),
            r#"
            export default {durable:{async work(ctx) { await ctx.sleep(1); return 42; }}};
        "#
            .into(),
            IsolateConfig { cpu_ms_per_turn: 500, request_timeout_ms: 1_000, ..Default::default() },
            "finalize",
        )
        .unwrap();
        let started: JsonValue =
            serde_json::from_str(&plane.start_named("work", "null").unwrap()).unwrap();
        let id = parse_task_id(started["taskId"].as_str().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !matches!(plane.health(), PollerHealth::Recovering { .. }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(store.wakeup(id).unwrap().is_none());
        assert!(store.completion(id).unwrap().is_none());
        assert!(plane.start_named("work", "null").is_err());
        // Replay would now fail. Only writing the retained outcome may succeed.
        connection.execute_batch("UPDATE durable_programs SET source = 'throw new Error(\"forbidden replay\")'; DROP TRIGGER fail_completion;").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while store.completion(id).unwrap().is_none() || plane.health() != PollerHealth::Healthy
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.completion(id).unwrap().unwrap().value, serde_json::json!(42));
        assert_eq!(store.program_count().unwrap(), 0);
        plane.shutdown().await.unwrap();
        drop(connection);
        drop(plane);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn corrupted_program_is_reported_to_service_owner() {
        let path = std::env::temp_dir().join(format!(
            "tysel-poller-fatal-{}-{}.db",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        store.put_module(TaskId(1), "export default async () => 1", 0).unwrap();
        store
            .schedule_wakeup(tysel_durable::Wakeup {
                task_id: TaskId(1),
                sequence: 0,
                wake_at_ms: 0,
            })
            .unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("UPDATE durable_programs SET source_sha256 = zeroblob(32)")
            .unwrap();
        let plane = DurablePlane::start(
            store.clone(),
            "export default {};".into(),
            IsolateConfig::default(),
            "fatal",
        )
        .unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), plane.failed()).await.unwrap();
        assert!(matches!(error, DurablePlaneError::Unavailable));
        assert_eq!(plane.health(), PollerHealth::Failed);
        assert!(plane.shutdown().await.is_err());
        assert!(store.wakeup(TaskId(1)).unwrap().is_some());
        drop(connection);
        drop(plane);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

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
