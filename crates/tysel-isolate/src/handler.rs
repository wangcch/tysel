use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tysel_engine::{HttpHead, HttpRequest, IsolateConfig};

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
