use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::task::JoinSet;
use tysel_durable::{
    DurableError, MAX_DURABLE_PROGRAM_BYTES, MAX_DURABLE_PROGRAM_TOTAL_BYTES, MAX_DURABLE_PROGRAMS,
    SqliteStore,
};
use tysel_task::TaskId;

use crate::{DispatchError, DurableDispatcher, DurableRun};

const MAX_POLL_BATCH: usize = 10_000;
const MAX_POLL_CONCURRENCY: usize = 16;

#[derive(Default)]
struct RegistryState {
    programs: HashMap<TaskId, Arc<str>>,
    total_bytes: usize,
}

#[derive(Clone, Default)]
pub struct DurableProgramRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl DurableProgramRegistry {
    pub fn register(
        &self,
        task_id: TaskId,
        script: impl Into<String>,
    ) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        let script = script.into();
        if script.is_empty() || script.len() > MAX_DURABLE_PROGRAM_BYTES {
            return Err(ProgramRegistryError::InvalidProgram);
        }
        let mut state = self.state.write().map_err(|_| ProgramRegistryError::Poisoned)?;
        if !state.programs.contains_key(&task_id) && state.programs.len() >= MAX_DURABLE_PROGRAMS {
            return Err(ProgramRegistryError::Full);
        }
        let replaced_bytes = state.programs.get(&task_id).map_or(0, |program| program.len());
        let total_bytes = next_total_bytes(state.total_bytes, replaced_bytes, script.len())?;
        let previous = state.programs.insert(task_id, Arc::from(script));
        state.total_bytes = total_bytes;
        Ok(previous)
    }

    pub fn resolve(&self, task_id: TaskId) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        let state = self.state.read().map_err(|_| ProgramRegistryError::Poisoned)?;
        Ok(state.programs.get(&task_id).cloned())
    }

    pub fn unregister(&self, task_id: TaskId) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        let mut state = self.state.write().map_err(|_| ProgramRegistryError::Poisoned)?;
        let removed = state.programs.remove(&task_id);
        if let Some(program) = &removed {
            state.total_bytes -= program.len();
        }
        Ok(removed)
    }

    pub fn len(&self) -> Result<usize, ProgramRegistryError> {
        let state = self.state.read().map_err(|_| ProgramRegistryError::Poisoned)?;
        Ok(state.programs.len())
    }

    pub fn is_empty(&self) -> Result<bool, ProgramRegistryError> {
        Ok(self.len()? == 0)
    }

    fn snapshot(&self) -> Result<Vec<(TaskId, Arc<str>)>, ProgramRegistryError> {
        let state = self.state.read().map_err(|_| ProgramRegistryError::Poisoned)?;
        let mut snapshot: Vec<_> =
            state.programs.iter().map(|(task_id, script)| (*task_id, script.clone())).collect();
        snapshot.sort_unstable_by_key(|(task_id, _)| *task_id);
        Ok(snapshot)
    }
}

fn next_total_bytes(
    current: usize,
    replaced: usize,
    incoming: usize,
) -> Result<usize, ProgramRegistryError> {
    let total = current
        .checked_sub(replaced)
        .and_then(|remaining| remaining.checked_add(incoming))
        .ok_or(ProgramRegistryError::TotalBytesExceeded)?;
    if total > MAX_DURABLE_PROGRAM_TOTAL_BYTES {
        return Err(ProgramRegistryError::TotalBytesExceeded);
    }
    Ok(total)
}

#[derive(Clone)]
pub struct DurableProgramCatalog {
    store: Arc<SqliteStore>,
}

impl DurableProgramCatalog {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }

    pub fn register(
        &self,
        task_id: TaskId,
        script: impl Into<String>,
    ) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        let script = script.into();
        let previous = self.store.put_program(task_id, &script, unix_time_ms()?)?;
        Ok(previous.map(|program| Arc::from(program.source)))
    }

    pub fn resolve(&self, task_id: TaskId) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        Ok(self.store.program(task_id)?.map(|program| Arc::from(program.source)))
    }

    pub fn unregister(&self, task_id: TaskId) -> Result<Option<Arc<str>>, ProgramRegistryError> {
        Ok(self.store.remove_program(task_id)?.map(|program| Arc::from(program.source)))
    }

    pub fn len(&self) -> Result<usize, ProgramRegistryError> {
        Ok(self.store.program_count()?)
    }

    pub fn is_empty(&self) -> Result<bool, ProgramRegistryError> {
        Ok(self.len()? == 0)
    }

    fn due_snapshot(&self, now_ms: u64) -> Result<Vec<(TaskId, Arc<str>)>, ProgramRegistryError> {
        self.store
            .load_due_programs(now_ms)?
            .into_iter()
            .map(|program| Ok((program.task_id, Arc::from(program.source))))
            .collect()
    }
}

#[derive(Clone)]
enum ProgramSource {
    Memory(DurableProgramRegistry),
    Persistent(DurableProgramCatalog),
}

impl ProgramSource {
    async fn snapshot(&self) -> Result<Vec<(TaskId, Arc<str>)>, PollerError> {
        match self {
            Self::Memory(registry) => Ok(registry.snapshot()?),
            Self::Persistent(catalog) => {
                let catalog = catalog.clone();
                tokio::task::spawn_blocking(move || catalog.due_snapshot(unix_time_ms()?))
                    .await
                    .map_err(PollerError::Join)?
                    .map_err(PollerError::Registry)
            }
        }
    }
}

pub struct DurablePoller {
    dispatcher: Arc<DurableDispatcher>,
    programs: ProgramSource,
    interval: Duration,
    batch_size: usize,
    cursor: AtomicUsize,
}

impl DurablePoller {
    pub fn new(
        dispatcher: Arc<DurableDispatcher>,
        programs: DurableProgramRegistry,
        interval: Duration,
        batch_size: usize,
    ) -> Result<Self, PollerError> {
        Self::with_programs(dispatcher, ProgramSource::Memory(programs), interval, batch_size)
    }

    /// Build a poller backed by the dispatcher's SQLite program catalog. The
    /// catalog is reopened with the durable store, so no in-memory repopulation
    /// is required after a process restart.
    pub fn new_persistent(
        dispatcher: Arc<DurableDispatcher>,
        interval: Duration,
        batch_size: usize,
    ) -> Result<Self, PollerError> {
        let catalog = DurableProgramCatalog::new(dispatcher.store());
        Self::with_programs(dispatcher, ProgramSource::Persistent(catalog), interval, batch_size)
    }

    fn with_programs(
        dispatcher: Arc<DurableDispatcher>,
        programs: ProgramSource,
        interval: Duration,
        batch_size: usize,
    ) -> Result<Self, PollerError> {
        if interval.is_zero() {
            return Err(PollerError::InvalidInterval);
        }
        if batch_size == 0 || batch_size > MAX_POLL_BATCH {
            return Err(PollerError::InvalidBatchSize);
        }
        Ok(Self { dispatcher, programs, interval, batch_size, cursor: AtomicUsize::new(0) })
    }

    pub async fn poll_once(&self) -> Result<Vec<DurableRun>, PollerError> {
        self.poll_batch(None).await
    }

    async fn poll_batch(
        &self,
        shutdown: Option<&PollerShutdown>,
    ) -> Result<Vec<DurableRun>, PollerError> {
        let dispatcher = self.dispatcher.clone();
        let mut programs = self.programs.snapshot().await?;
        let batch_size = self.batch_size;
        if !programs.is_empty() {
            let start = self.cursor.fetch_add(batch_size, Ordering::Relaxed) % programs.len();
            programs.rotate_left(start);
        }
        let mut programs = programs.into_iter();
        let mut pending = JoinSet::new();
        let mut runs = Vec::with_capacity(batch_size.min(programs.len()));
        loop {
            let cancelled = shutdown.is_some_and(PollerShutdown::is_cancelled);
            while !cancelled
                && pending.len() < MAX_POLL_CONCURRENCY
                && runs.len() + pending.len() < batch_size
            {
                let Some((task_id, script)) = programs.next() else {
                    break;
                };
                let dispatcher = dispatcher.clone();
                pending.spawn_blocking(move || dispatcher.dispatch_task(task_id, &script));
            }

            let Some(joined) = pending.join_next().await else {
                break;
            };
            if let Some(run) = joined.map_err(PollerError::Join)?.map_err(PollerError::Dispatch)? {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    /// Poll immediately, then at `interval` until shutdown. An in-flight batch
    /// finishes before the loop exits so a claimed task is never abandoned.
    pub async fn run<F>(&self, shutdown: PollerShutdown, mut on_run: F) -> Result<(), PollerError>
    where
        F: FnMut(DurableRun) + Send,
    {
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            for run in self.poll_batch(Some(&shutdown)).await? {
                on_run(run);
            }
            tokio::select! {
                () = shutdown.cancelled() => return Ok(()),
                () = tokio::time::sleep(self.interval) => {}
            }
        }
    }
}

#[derive(Clone)]
pub struct PollerShutdown {
    sender: watch::Sender<bool>,
}

impl Default for PollerShutdown {
    fn default() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }
}

impl PollerShutdown {
    pub fn cancel(&self) {
        self.sender.send_if_modified(|cancelled| {
            let changed = !*cancelled;
            *cancelled = true;
            changed
        });
    }

    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProgramRegistryError {
    #[error("durable program must be 1..={MAX_DURABLE_PROGRAM_BYTES} bytes")]
    InvalidProgram,
    #[error("durable program registry exceeds {MAX_DURABLE_PROGRAMS} tasks")]
    Full,
    #[error("durable program registry exceeds {MAX_DURABLE_PROGRAM_TOTAL_BYTES} total bytes")]
    TotalBytesExceeded,
    #[error("durable program registry lock is poisoned")]
    Poisoned,
    #[error(transparent)]
    Store(#[from] DurableError),
    #[error("system clock is before the Unix epoch")]
    SystemClock,
    #[error("system time is too large")]
    TimeRange,
}

fn unix_time_ms() -> Result<u64, ProgramRegistryError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProgramRegistryError::SystemClock)?
        .as_millis();
    u64::try_from(millis).map_err(|_| ProgramRegistryError::TimeRange)
}

#[derive(Debug, thiserror::Error)]
pub enum PollerError {
    #[error("durable poll interval must be greater than zero")]
    InvalidInterval,
    #[error("durable poll batch must be 1..={MAX_POLL_BATCH}")]
    InvalidBatchSize,
    #[error(transparent)]
    Registry(#[from] ProgramRegistryError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error("durable polling worker failed: {0}")]
    Join(tokio::task::JoinError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tysel_durable::SqliteStore;
    use tysel_engine::{IsolateConfig, Value};

    const WAIT_SCRIPT: &str = r#"
        (async () => JSON.stringify(await tysel.durable.waitForSignal("approval")))()
    "#;

    fn dispatcher(store: Arc<SqliteStore>, owner: &str) -> Arc<DurableDispatcher> {
        Arc::new(
            DurableDispatcher::new(
                store,
                owner,
                1_000,
                IsolateConfig {
                    request_timeout_ms: 10,
                    cpu_ms_per_turn: 50,
                    memory_limit_bytes: 8 * 1024 * 1024,
                },
            )
            .unwrap(),
        )
    }

    fn unix_time_ms() -> u64 {
        u64::try_from(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        )
        .unwrap()
    }

    #[test]
    fn registry_updates_and_removes_bounded_programs() {
        let registry = DurableProgramRegistry::default();
        let id = TaskId(301);
        assert!(registry.is_empty().unwrap());
        assert!(matches!(registry.register(id, ""), Err(ProgramRegistryError::InvalidProgram)));
        assert_eq!(registry.register(id, "one").unwrap(), None);
        assert_eq!(registry.resolve(id).unwrap().unwrap().as_ref(), "one");
        assert_eq!(registry.register(id, "two").unwrap().unwrap().as_ref(), "one");
        assert_eq!(registry.len().unwrap(), 1);
        assert_eq!(registry.unregister(id).unwrap().unwrap().as_ref(), "two");
        assert!(registry.is_empty().unwrap());
    }

    #[test]
    fn registry_tracks_replacements_and_enforces_the_aggregate_budget() {
        assert_eq!(next_total_bytes(10, 4, 7).unwrap(), 13);
        assert!(matches!(
            next_total_bytes(MAX_DURABLE_PROGRAM_TOTAL_BYTES, 0, 1),
            Err(ProgramRegistryError::TotalBytesExceeded)
        ));

        let registry = DurableProgramRegistry::default();
        registry.register(TaskId(307), "1234").unwrap();
        registry.register(TaskId(308), "123456").unwrap();
        registry.register(TaskId(307), "12").unwrap();
        assert_eq!(registry.state.read().unwrap().total_bytes, 8);
        registry.unregister(TaskId(308)).unwrap();
        assert_eq!(registry.state.read().unwrap().total_bytes, 2);
    }

    #[tokio::test]
    async fn unknown_tasks_are_not_claimed_or_allowed_to_starve_registered_work() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let setup = dispatcher(store.clone(), "setup");
        let unknown = TaskId(302);
        let registered = TaskId(303);
        assert!(matches!(
            setup.start(unknown, WAIT_SCRIPT).result,
            Ok(crate::DurableRunStatus::Suspended)
        ));
        assert!(matches!(
            setup.start(registered, WAIT_SCRIPT).result,
            Ok(crate::DurableRunStatus::Suspended)
        ));
        let now_ms = unix_time_ms();
        store.send_signal(unknown, "approval", &serde_json::json!("unknown"), now_ms).unwrap();
        store.send_signal(registered, "approval", &serde_json::json!("known"), now_ms).unwrap();

        let programs = DurableProgramRegistry::default();
        let poller = DurablePoller::new(
            dispatcher(store.clone(), "poller"),
            programs.clone(),
            Duration::from_millis(10),
            1,
        )
        .unwrap();
        assert!(poller.poll_once().await.unwrap().is_empty());
        programs.register(registered, WAIT_SCRIPT).unwrap();
        let runs = poller.poll_once().await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].task_id, registered);
        assert_eq!(
            runs[0].result.as_ref().unwrap(),
            &crate::DurableRunStatus::Completed(Value::String(r#""known""#.into()))
        );
        let probe = store.claim_wakeup(unknown, unix_time_ms(), "probe", 100).unwrap().unwrap();
        assert!(store.release_wakeup_claim(&probe).unwrap());
    }

    #[tokio::test]
    async fn persistent_catalog_resumes_work_after_store_reopen() {
        let path = std::env::temp_dir().join(format!(
            "tysel-runtime-programs-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = TaskId(309);
        {
            let store = Arc::new(SqliteStore::open(&path).unwrap());
            let setup = dispatcher(store.clone(), "setup");
            let catalog = DurableProgramCatalog::new(store.clone());
            assert_eq!(catalog.register(id, WAIT_SCRIPT).unwrap(), None);
            assert!(matches!(
                setup.start(id, WAIT_SCRIPT).result,
                Ok(crate::DurableRunStatus::Suspended)
            ));
            store
                .send_signal(id, "approval", &serde_json::json!("restarted"), unix_time_ms())
                .unwrap();
        }

        let reopened = Arc::new(SqliteStore::open(&path).unwrap());
        let poller = DurablePoller::new_persistent(
            dispatcher(reopened.clone(), "restarted-poller"),
            Duration::from_millis(10),
            8,
        )
        .unwrap();
        let runs = poller.poll_once().await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].task_id, id);
        assert_eq!(
            runs[0].result.as_ref().unwrap(),
            &crate::DurableRunStatus::Completed(Value::String(r#""restarted""#.into()))
        );
        drop(poller);
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn round_robin_cursor_prevents_a_failing_task_from_starving_the_batch() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let setup = dispatcher(store.clone(), "setup");
        let failing = TaskId(305);
        let healthy = TaskId(306);
        for id in [failing, healthy] {
            assert!(matches!(
                setup.start(id, WAIT_SCRIPT).result,
                Ok(crate::DurableRunStatus::Suspended)
            ));
            store.send_signal(id, "approval", &serde_json::json!(true), unix_time_ms()).unwrap();
        }
        let programs = DurableProgramRegistry::default();
        programs.register(failing, "throw new Error('changed')").unwrap();
        programs.register(healthy, WAIT_SCRIPT).unwrap();
        let poller =
            DurablePoller::new(dispatcher(store, "poller"), programs, Duration::from_millis(10), 1)
                .unwrap();

        let first = poller.poll_once().await.unwrap();
        assert_eq!(first[0].task_id, failing);
        assert!(first[0].result.is_err());
        let second = poller.poll_once().await.unwrap();
        assert_eq!(second[0].task_id, healthy);
        assert!(matches!(second[0].result, Ok(crate::DurableRunStatus::Completed(_))));
    }

    #[tokio::test]
    async fn polling_loop_reports_runs_and_stops() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let dispatcher = dispatcher(store.clone(), "poller");
        let id = TaskId(304);
        assert!(matches!(
            dispatcher.start(id, WAIT_SCRIPT).result,
            Ok(crate::DurableRunStatus::Suspended)
        ));
        store.send_signal(id, "approval", &serde_json::json!(true), unix_time_ms()).unwrap();
        let programs = DurableProgramRegistry::default();
        programs.register(id, WAIT_SCRIPT).unwrap();
        let poller = DurablePoller::new(dispatcher, programs, Duration::from_secs(60), 8).unwrap();
        let shutdown = PollerShutdown::default();
        let stop_from_callback = shutdown.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            poller
                .run(shutdown, move |run| {
                    tx.send(run).unwrap();
                    stop_from_callback.cancel();
                })
                .await
        });
        let run = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
        assert_eq!(run.task_id, id);
        tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap().unwrap();
    }

    #[tokio::test]
    async fn idle_poller_shutdown_does_not_wait_for_the_interval() {
        let poller = DurablePoller::new(
            dispatcher(Arc::new(SqliteStore::in_memory().unwrap()), "poller"),
            DurableProgramRegistry::default(),
            Duration::from_secs(60),
            8,
        )
        .unwrap();
        let shutdown = PollerShutdown::default();
        let stop = shutdown.clone();
        let task = tokio::spawn(async move { poller.run(shutdown, |_| {}).await });
        tokio::task::yield_now().await;
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap().unwrap();
    }

    #[tokio::test]
    async fn shutdown_stops_scheduling_after_the_bounded_in_flight_set() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let setup = dispatcher(store.clone(), "setup");
        let programs = DurableProgramRegistry::default();
        for offset in 0..20 {
            let id = TaskId(400 + offset);
            assert!(matches!(
                setup.start(id, WAIT_SCRIPT).result,
                Ok(crate::DurableRunStatus::Suspended)
            ));
            store.send_signal(id, "approval", &serde_json::json!(true), unix_time_ms()).unwrap();
            programs.register(id, "(async () => tysel.sleep(5000))()").unwrap();
        }
        let slow_dispatcher = Arc::new(
            DurableDispatcher::new(
                store,
                "poller",
                1_000,
                IsolateConfig {
                    request_timeout_ms: 100,
                    cpu_ms_per_turn: 50,
                    memory_limit_bytes: 8 * 1024 * 1024,
                },
            )
            .unwrap(),
        );
        let poller =
            DurablePoller::new(slow_dispatcher, programs, Duration::from_secs(60), 20).unwrap();
        let shutdown = PollerShutdown::default();
        let stop = shutdown.clone();
        let completed = Arc::new(AtomicUsize::new(0));
        let completed_in_callback = completed.clone();
        let task = tokio::spawn(async move {
            poller
                .run(shutdown, move |_| {
                    completed_in_callback.fetch_add(1, Ordering::Relaxed);
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap().unwrap();
        assert_eq!(completed.load(Ordering::Relaxed), MAX_POLL_CONCURRENCY);
    }
}
