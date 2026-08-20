//! Lifecycle owner for the local TaskRPC broker, module worker and cron poller.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tysel_cap_mcp::{McpDispatch, McpServer};
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::inspect_task_module;
use tysel_task_rpc::TaskOutcome;

use crate::{
    TaskIngress, TaskIngressError, TaskModuleWorker, TaskModuleWorkerError, TaskRegistry,
    TaskRpcBroker, TaskRpcServerError, TaskRpcServerShutdown, serve_task_rpc_unix,
};

const DEFAULT_QUEUE_CAPACITY: usize = 1_024;
const IDLE_WORKER_DELAY: Duration = Duration::from_millis(10);
const CRON_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Running local task plane. Queue producers use [`Self::ingress`]; dropping
/// the owner requests cooperative shutdown, while [`Self::shutdown`] also
/// waits for all background loops and removes the Unix socket.
pub struct ModuleTaskService {
    ingress: Arc<TaskIngress>,
    shutdown: TaskRpcServerShutdown,
    tasks: Vec<JoinHandle<Result<(), ModuleTaskServiceError>>>,
    socket_path: PathBuf,
}

impl ModuleTaskService {
    pub async fn start(
        socket_path: impl AsRef<Path>,
        application_id: impl Into<String>,
        source: impl Into<String>,
        config: IsolateConfig,
    ) -> Result<Self, ModuleTaskServiceError> {
        Self::start_with_capacity(
            socket_path,
            application_id,
            source,
            config,
            DEFAULT_QUEUE_CAPACITY,
        )
        .await
    }

    pub async fn start_with_capacity(
        socket_path: impl AsRef<Path>,
        application_id: impl Into<String>,
        source: impl Into<String>,
        config: IsolateConfig,
        queue_capacity: usize,
    ) -> Result<Self, ModuleTaskServiceError> {
        let socket_path = socket_path.as_ref().to_owned();
        let source = source.into();
        let inspection_source = source.clone();
        let definitions =
            tokio::task::spawn_blocking(move || inspect_task_module(&inspection_source, config))
                .await??;
        let registry = TaskRegistry::from_definitions(&definitions)?;
        let broker = Arc::new(Mutex::new(TaskRpcBroker::new(queue_capacity)?));
        let seed = task_id_seed()?;
        let ingress = Arc::new(TaskIngress::new(
            Arc::clone(&broker),
            registry,
            application_id,
            seed,
            config.request_timeout_ms.max(1),
        )?);

        let listener = UnixListener::bind(&socket_path).map_err(|source| {
            ModuleTaskServiceError::Socket { path: socket_path.clone(), source }
        })?;
        let shutdown = TaskRpcServerShutdown::new();
        let server_shutdown = shutdown.clone();
        let server = tokio::spawn(async move {
            let result = serve_task_rpc_unix(listener, broker, server_shutdown.clone()).await;
            if result.is_err() {
                server_shutdown.cancel();
            }
            result.map_err(ModuleTaskServiceError::Server)
        });

        let lease_ms = config
            .request_timeout_ms
            .max(1)
            .checked_add(5_000)
            .ok_or(ModuleTaskServiceError::LeaseOverflow)?;
        let worker = match TaskModuleWorker::connect(
            &socket_path,
            format!("module-{}", std::process::id()),
            source,
            config,
            lease_ms,
        )
        .await
        {
            Ok(worker) => worker,
            Err(error) => {
                shutdown.cancel();
                let _ = server.await;
                let _ = std::fs::remove_file(&socket_path);
                return Err(error.into());
            }
        };

        let worker_shutdown = shutdown.clone();
        let worker_task = tokio::spawn(async move {
            let result = run_worker(worker, worker_shutdown.clone()).await;
            if result.is_err() {
                worker_shutdown.cancel();
            }
            result
        });
        let cron_shutdown = shutdown.clone();
        let cron_ingress = Arc::clone(&ingress);
        let cron_task = tokio::spawn(async move {
            run_cron(cron_ingress, cron_shutdown).await;
            Ok(())
        });

        Ok(Self { ingress, shutdown, tasks: vec![server, worker_task, cron_task], socket_path })
    }

    pub fn ingress(&self) -> Arc<TaskIngress> {
        Arc::clone(&self.ingress)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn mcp_endpoint(&self) -> Result<McpTaskEndpoint, ModuleTaskServiceError> {
        McpTaskEndpoint::new(Arc::clone(&self.ingress))
    }

    pub async fn shutdown(mut self) -> Result<(), ModuleTaskServiceError> {
        self.shutdown.cancel();
        let mut first_error = None;
        for task in self.tasks.drain(..) {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
                Err(error) if first_error.is_none() => {
                    first_error = Some(ModuleTaskServiceError::Background(error));
                }
                Ok(Err(_)) | Err(_) => {}
            }
        }
        remove_socket(&self.socket_path)?;
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct McpTaskEndpoint {
    server: McpServer,
    ingress: Arc<TaskIngress>,
}

impl McpTaskEndpoint {
    pub fn new(ingress: Arc<TaskIngress>) -> Result<Self, ModuleTaskServiceError> {
        let tools = ingress.registry().mcp_tools().cloned().collect::<Vec<_>>();
        let server = McpServer::new("tysel", env!("CARGO_PKG_VERSION"), tools)?;
        Ok(Self { server, ingress })
    }

    /// Handle one decoded MCP request. Notifications return `None`; a tool call
    /// is queued and its fenced terminal outcome becomes the MCP result.
    pub async fn handle(&self, request: serde_json::Value) -> Option<serde_json::Value> {
        self.handle_dispatch(self.server.handle(request)).await
    }

    pub async fn handle_bytes(
        &self,
        request: &[u8],
    ) -> Result<Option<serde_json::Value>, tysel_cap_mcp::McpError> {
        Ok(self.handle_dispatch(self.server.handle_bytes(request)?).await)
    }

    async fn handle_dispatch(&self, dispatch: McpDispatch) -> Option<serde_json::Value> {
        match dispatch {
            McpDispatch::Response(response) => Some(response),
            McpDispatch::Notification => None,
            McpDispatch::ToolCall(call) => {
                let now = match unix_ms() {
                    Ok(now) => now,
                    Err(error) => {
                        return Some(self.server.fail_tool_call(call, &error.to_string()));
                    }
                };
                let id =
                    match self.ingress.enqueue_mcp(&call.name, call.arguments.clone(), now).await {
                        Ok(id) => id,
                        Err(error) => {
                            return Some(self.server.fail_tool_call(call, &error.to_string()));
                        }
                    };
                let outcome = tokio::time::timeout(
                    Duration::from_millis(self.ingress.request_timeout_ms()),
                    wait_for_outcome(&self.ingress, id),
                )
                .await;
                match outcome {
                    Ok(TaskOutcome::Completed { result }) => {
                        Some(self.server.complete_tool_call(call, result))
                    }
                    Ok(TaskOutcome::Failed { error, .. }) => {
                        Some(self.server.fail_tool_call(call, &error))
                    }
                    Ok(TaskOutcome::Canceled {}) => {
                        Some(self.server.fail_tool_call(call, "tool call was canceled"))
                    }
                    Ok(TaskOutcome::TimedOut {}) | Err(_) => {
                        Some(self.server.fail_tool_call(call, "tool call timed out"))
                    }
                    Ok(TaskOutcome::Suspended {}) => {
                        Some(self.server.fail_tool_call(call, "tool call suspended"))
                    }
                }
            }
        }
    }
}

async fn wait_for_outcome(ingress: &TaskIngress, id: tysel_task::TaskId) -> TaskOutcome {
    loop {
        if let Some(outcome) = ingress.outcome(id).await {
            return outcome;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

impl Drop for ModuleTaskService {
    fn drop(&mut self) {
        self.shutdown.cancel();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn run_worker(
    mut worker: TaskModuleWorker,
    shutdown: TaskRpcServerShutdown,
) -> Result<(), ModuleTaskServiceError> {
    loop {
        let ran = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(()),
            result = worker.run_once() => result?,
        };
        if !ran {
            tokio::select! {
                () = shutdown.cancelled() => return Ok(()),
                () = tokio::time::sleep(IDLE_WORKER_DELAY) => {}
            }
        }
    }
}

async fn run_cron(ingress: Arc<TaskIngress>, shutdown: TaskRpcServerShutdown) {
    let mut interval = tokio::time::interval(CRON_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            _ = interval.tick() => {
                let result = match unix_ms() {
                    Ok(now) => ingress.enqueue_due_cron(now).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(triggered) => {
                        for task in triggered {
                            tracing::info!(
                                task_id = %task.id,
                                handler = %task.handler,
                                scheduled_at_ms = task.scheduled_at_ms,
                                "cron task queued"
                            );
                        }
                    }
                    Err(error) => tracing::warn!(error = %error, "cron poll deferred"),
                }
            }
        }
    }
}

fn unix_ms() -> Result<u64, TaskIngressError> {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| TaskIngressError::Clock)?
        .as_millis();
    u64::try_from(millis).map_err(|_| TaskIngressError::Clock)
}

fn task_id_seed() -> Result<u64, ModuleTaskServiceError> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| ModuleTaskServiceError::Clock)?
        .as_nanos();
    let folded = (nanos ^ (nanos >> 64)) as u64;
    Ok(folded.max(1).min(u64::MAX - DEFAULT_QUEUE_CAPACITY as u64 - 1))
}

fn remove_socket(path: &Path) -> Result<(), ModuleTaskServiceError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ModuleTaskServiceError::Socket { path: path.to_owned(), source }),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleTaskServiceError {
    #[error(transparent)]
    Engine(#[from] tysel_engine::EngineError),
    #[error("task module inspection panicked or was canceled: {0}")]
    Inspection(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Ingress(#[from] TaskIngressError),
    #[error(transparent)]
    Scheduler(#[from] tysel_scheduler::SchedulerError),
    #[error(transparent)]
    Worker(#[from] TaskModuleWorkerError),
    #[error(transparent)]
    Server(#[from] TaskRpcServerError),
    #[error(transparent)]
    Mcp(#[from] tysel_cap_mcp::McpError),
    #[error("task service background task panicked or was canceled: {0}")]
    Background(tokio::task::JoinError),
    #[error("failed to use task socket {path}: {source}")]
    Socket { path: PathBuf, source: std::io::Error },
    #[error("task lease duration overflow")]
    LeaseOverflow,
    #[error("system clock is before the Unix epoch")]
    Clock,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "tysel-task-service-{}-{}.sock",
            std::process::id(),
            task_id_seed().unwrap()
        ))
    }

    #[tokio::test]
    async fn service_executes_queue_work_and_removes_its_socket() {
        let path = socket_path();
        let source = r#"
export default {
  tasks: {
    consume: {
      kind: "queue",
      name: "orders",
      handler(input, ctx) { return { order: input.order, requestId: ctx.requestId }; }
    },
    analyze: {
      kind: "mcp",
      description: "Analyze a customer",
      input: { customerId: "string" },
      handler(input) { return { customer: input.customerId, risk: "low" }; }
    }
  }
};
"#;
        let service = ModuleTaskService::start_with_capacity(
            &path,
            "test-app",
            source,
            IsolateConfig {
                memory_limit_bytes: 16 * 1024 * 1024,
                cpu_ms_per_turn: 50,
                request_timeout_ms: 1_000,
            },
            4,
        )
        .await
        .unwrap();
        let ingress = service.ingress();
        let id = ingress
            .enqueue_queue(
                "orders",
                Some("message-1".into()),
                serde_json::json!({"order": 9}),
                unix_ms().unwrap(),
            )
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if ingress.outcome(id).await.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("queue task should finish");

        let endpoint = service.mcp_endpoint().unwrap();
        let response = endpoint
            .handle(serde_json::json!({
                "jsonrpc": "2.0",
                "id": "call-1",
                "method": "tools/call",
                "params": {
                    "name": "analyze",
                    "arguments": { "customerId": "customer-9" },
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2026-07-28"
                    }
                }
            }))
            .await
            .unwrap();
        assert_eq!(response["result"]["structuredContent"]["customer"], "customer-9");
        assert_eq!(response["result"]["isError"], false);
        assert!(path.exists());
        service.shutdown().await.unwrap();
        assert!(!path.exists());
    }
}
