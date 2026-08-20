use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tysel_engine::{HttpHead, HttpRequest, IsolateConfig, Value};
use tysel_engine_qjs::ModuleTaskDefinition;

use crate::supervisor::{IsolateError, Supervisor, WorkerSpec};

fn spec_from_config(config: IsolateConfig) -> WorkerSpec {
    let (app, json_logs) = tysel_observability::json_log_state();
    WorkerSpec { app, json_logs, ..WorkerSpec::from(config) }
}

/// Fetch-handler pool that runs QuickJS in `tysel-worker`. The supervisor keeps
/// the HTTP listener and secret values; the worker only sees secret names.
pub struct IsolatedHttpPool {
    inner: Mutex<Supervisor>,
}

/// Module-task executor backed by a sandboxed `tysel-worker` process. Module
/// inspection and every invocation cross the same bounded IPC boundary as HTTP.
pub struct IsolatedTaskPool {
    inner: Mutex<Supervisor>,
}

impl IsolatedTaskPool {
    pub fn spawn_from_config(
        worker_bin: impl AsRef<Path>,
        source: &str,
        config: IsolateConfig,
        secret_names: Vec<String>,
    ) -> Result<(Self, Vec<ModuleTaskDefinition>), IsolateError> {
        let (app, json_logs) = tysel_observability::json_log_state();
        let spec = WorkerSpec { app, json_logs, ..WorkerSpec::from(config) };
        let mut supervisor = Supervisor::spawn(worker_bin, spec, HashMap::new())?;
        let definitions = supervisor.load_task_module(source, secret_names)?;
        Ok((Self { inner: Mutex::new(supervisor) }, definitions))
    }

    pub fn invoke_sync(
        &self,
        task_name: &str,
        input_json: &str,
        request_id: &str,
        deadline_ms: u64,
    ) -> Result<Value, IsolateError> {
        self.inner.lock().map_err(|error| IsolateError::Worker(error.to_string()))?.invoke_task(
            task_name,
            input_json,
            request_id,
            deadline_ms,
        )
    }

    pub fn kill_worker(&self) -> Result<(), IsolateError> {
        self.inner.lock().map_err(|error| IsolateError::Worker(error.to_string()))?.kill_worker()
    }
}

impl IsolatedHttpPool {
    pub fn spawn(
        worker_bin: impl AsRef<Path>,
        source: &str,
        spec: WorkerSpec,
        secret_names: Vec<String>,
    ) -> Result<Self, IsolateError> {
        let secrets = secret_names
            .iter()
            .cloned()
            .map(|name| (name, String::new()))
            .collect::<HashMap<_, _>>();
        let mut supervisor = Supervisor::spawn(worker_bin, spec, secrets)?;
        supervisor.load_handler(source, secret_names)?;
        Ok(Self { inner: Mutex::new(supervisor) })
    }

    pub fn spawn_from_config(
        worker_bin: impl AsRef<Path>,
        source: &str,
        config: IsolateConfig,
        secret_names: Vec<String>,
    ) -> Result<Self, IsolateError> {
        Self::spawn(worker_bin, source, spec_from_config(config), secret_names)
    }

    pub fn dispatch_sync(&self, request: HttpRequest) -> Result<(HttpHead, Vec<u8>), IsolateError> {
        self.inner.lock().map_err(|err| IsolateError::Worker(err.to_string()))?.http(&request)
    }
}
