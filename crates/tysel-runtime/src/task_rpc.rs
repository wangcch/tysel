use std::collections::HashMap;

#[cfg(unix)]
use std::{collections::HashSet, path::Path, sync::Arc, time::SystemTime};

#[cfg(unix)]
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, watch},
    task::JoinSet,
};

#[cfg(unix)]
use tysel_engine::{EngineError, InterruptReason, IsolateConfig, Value};
#[cfg(unix)]
use tysel_engine_qjs::{inspect_task_module, invoke_task_module};
use tysel_scheduler::{Scheduler, SchedulerError, TaskClaim};
use tysel_task::{Task, TaskId, TaskState};
use tysel_task_rpc::{
    Envelope, ErrorCode, LeaseToken, Message, TaskDescriptor, TaskLease, TaskOutcome, TaskRpcError,
    WireTaskId, encode_message, read_message, write_message,
};

#[cfg(unix)]
use tysel_task_rpc::{MAX_TASK_RPC_FRAME, decode_message};

/// In-process TaskRPC coordinator. Transport code can decode a bounded
/// [`Envelope`], pass it here, and encode the returned response without gaining
/// direct access to scheduler state.
pub struct TaskRpcBroker {
    scheduler: Scheduler,
    outcomes: HashMap<TaskId, TaskOutcome>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskRpcBrokerError {
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error(transparent)]
    Protocol(#[from] TaskRpcError),
}

impl TaskRpcBroker {
    pub fn new(capacity: usize) -> Result<Self, SchedulerError> {
        Ok(Self { scheduler: Scheduler::new(capacity)?, outcomes: HashMap::new() })
    }

    pub fn enqueue(&mut self, task: Task) -> Result<(), TaskRpcBrokerError> {
        // Reject tasks that cannot be represented on the wire before they can
        // leave the runnable queue under an undeliverable claim.
        let mut claimed_shape = task.clone();
        claimed_shape.transition(TaskState::Queued).map_err(SchedulerError::from)?;
        claimed_shape.begin_attempt().map_err(SchedulerError::from)?;
        TaskDescriptor::from_task(&claimed_shape)?;
        let worst_lease =
            preview_wire_lease(&claimed_shape, &"w".repeat(tysel_task_rpc::MAX_WORKER_ID_BYTES))?;
        encode_message(&Envelope::new(Message::Claimed {
            request_id: u64::MAX,
            leases: vec![worst_lease],
        }))?;
        self.scheduler.enqueue(task)?;
        Ok(())
    }

    pub fn task(&self, task_id: TaskId) -> Option<&Task> {
        self.scheduler.get(task_id)
    }

    pub fn outcome(&self, task_id: TaskId) -> Option<&TaskOutcome> {
        self.outcomes.get(&task_id)
    }

    pub fn requeue_expired(
        &mut self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        self.scheduler.requeue_expired(now_ms, limit)
    }

    pub fn disconnect_worker(
        &mut self,
        worker_id: &str,
        now_ms: u64,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        self.scheduler.requeue_owner_claims(worker_id, now_ms, usize::MAX)
    }

    /// Read, validate, handle, and write one bounded TaskRPC frame.
    pub fn handle_frame(
        &mut self,
        now_ms: u64,
        reader: &mut impl std::io::Read,
        writer: &mut impl std::io::Write,
    ) -> Result<(), TaskRpcError> {
        let request = read_message(reader)?;
        write_message(writer, &self.handle(now_ms, request))
    }

    /// Validate and execute one TaskRPC request. Protocol errors are returned as
    /// bounded v1 `error` messages; stale leases use the operation's explicit
    /// negative acknowledgement so workers can safely discard late results.
    pub fn handle(&mut self, now_ms: u64, envelope: Envelope) -> Envelope {
        let request_id = request_id(&envelope.message);
        if let Err(error) = envelope.validate() {
            let code = if matches!(error, TaskRpcError::UnsupportedVersion { .. }) {
                ErrorCode::VersionMismatch
            } else {
                ErrorCode::InvalidRequest
            };
            return error_response(request_id, code, error.to_string());
        }

        let response = match envelope.message {
            Message::Hello { .. } => Message::Ready {},
            Message::Claim { request_id, worker_id, lease_ms, limit } => {
                self.claim(request_id, &worker_id, lease_ms, limit, now_ms)
            }
            Message::Renew { request_id, lease, lease_ms } => {
                self.renew(request_id, &lease, lease_ms, now_ms)
            }
            Message::Release { request_id, lease } => self.release(request_id, &lease, now_ms),
            Message::Cancel { request_id, task_id } => self.cancel(request_id, &task_id),
            Message::Commit { request_id, lease, outcome } => {
                self.commit(request_id, &lease, outcome, now_ms)
            }
            Message::Ready {}
            | Message::Claimed { .. }
            | Message::Renewed { .. }
            | Message::Released { .. }
            | Message::Canceled { .. }
            | Message::Committed { .. }
            | Message::Error { .. } => error_message(
                request_id,
                ErrorCode::InvalidRequest,
                "TaskRPC response sent to scheduler",
            ),
        };
        Envelope::new(response)
    }

    fn claim(
        &mut self,
        request_id: u64,
        worker_id: &str,
        lease_ms: u64,
        limit: u16,
        now_ms: u64,
    ) -> Message {
        let mut leases = Vec::with_capacity(usize::from(limit));
        if let Err(error) = self.scheduler.requeue_expired(now_ms, usize::from(limit)) {
            return scheduler_error(Some(request_id), error);
        }
        for _ in 0..limit {
            let preview = match self.scheduler.peek_runnable(now_ms) {
                Ok(Some(mut task)) => {
                    if task.begin_attempt().is_err() {
                        return error_message(
                            Some(request_id),
                            ErrorCode::Internal,
                            "queued task cannot begin an attempt",
                        );
                    }
                    match preview_wire_lease(&task, worker_id) {
                        Ok(lease) => lease,
                        Err(error) => {
                            return error_message(
                                Some(request_id),
                                ErrorCode::Internal,
                                &error.to_string(),
                            );
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => return scheduler_error(Some(request_id), error),
            };
            let mut candidate = leases.clone();
            candidate.push(preview);
            match encode_message(&Envelope::new(Message::Claimed { request_id, leases: candidate }))
            {
                Ok(_) => {}
                Err(TaskRpcError::FrameTooLarge(_)) if !leases.is_empty() => break,
                Err(error) => {
                    return error_message(
                        Some(request_id),
                        ErrorCode::Internal,
                        &error.to_string(),
                    );
                }
            }
            match self.scheduler.claim_with_lease(now_ms, worker_id, lease_ms) {
                Ok(Some(claim)) => match wire_lease(&claim) {
                    Ok(lease) => leases.push(lease),
                    Err(error) => {
                        let _ = self.scheduler.release_claim(&claim, now_ms);
                        return error_message(
                            Some(request_id),
                            ErrorCode::Internal,
                            &error.to_string(),
                        );
                    }
                },
                Ok(None) => break,
                Err(_) if !leases.is_empty() => break,
                Err(error) => return scheduler_error(Some(request_id), error),
            }
        }
        Message::Claimed { request_id, leases }
    }

    fn renew(
        &mut self,
        request_id: u64,
        lease: &LeaseToken,
        lease_ms: u64,
        now_ms: u64,
    ) -> Message {
        let claim = match self.claim_from_token(lease) {
            Ok(claim) => claim,
            Err(_) => return Message::Renewed { request_id, lease: None },
        };
        match self.scheduler.renew_claim(&claim, now_ms, lease_ms) {
            Ok(claim) => match wire_lease(&claim) {
                Ok(lease) => Message::Renewed { request_id, lease: Some(lease) },
                Err(error) => {
                    error_message(Some(request_id), ErrorCode::Internal, &error.to_string())
                }
            },
            Err(SchedulerError::LeaseLost | SchedulerError::Unknown(_)) => {
                Message::Renewed { request_id, lease: None }
            }
            Err(error) => scheduler_error(Some(request_id), error),
        }
    }

    fn release(&mut self, request_id: u64, lease: &LeaseToken, now_ms: u64) -> Message {
        let claim = match self.claim_from_token(lease) {
            Ok(claim) => claim,
            Err(_) => return Message::Released { request_id, released: false },
        };
        match self.scheduler.release_claim(&claim, now_ms) {
            Ok(_) => Message::Released { request_id, released: true },
            Err(
                SchedulerError::LeaseLost
                | SchedulerError::Unknown(_)
                | SchedulerError::Full { .. },
            ) => Message::Released { request_id, released: false },
            Err(error) => scheduler_error(Some(request_id), error),
        }
    }

    fn cancel(&mut self, request_id: u64, task_id: &WireTaskId) -> Message {
        let Ok(task_id) = task_id.parse() else {
            return Message::Canceled { request_id, canceled: false };
        };
        match self.scheduler.cancel(task_id) {
            Ok(task) => {
                if task.state == TaskState::Canceled {
                    self.outcomes.insert(task_id, TaskOutcome::Canceled {});
                }
                Message::Canceled { request_id, canceled: task.state == TaskState::Canceled }
            }
            Err(SchedulerError::Unknown(_)) => Message::Canceled { request_id, canceled: false },
            Err(error) => scheduler_error(Some(request_id), error),
        }
    }

    fn commit(
        &mut self,
        request_id: u64,
        lease: &LeaseToken,
        outcome: TaskOutcome,
        now_ms: u64,
    ) -> Message {
        let claim = match self.claim_from_token(lease) {
            Ok(claim) => claim,
            Err(_) => return Message::Committed { request_id, accepted: false },
        };
        let task_id = claim.task.meta.id;
        let result = match &outcome {
            TaskOutcome::Completed { .. } => {
                self.scheduler.finish_claim(&claim, now_ms, TaskState::Completed)
            }
            TaskOutcome::Suspended {} => {
                self.scheduler.finish_claim(&claim, now_ms, TaskState::Suspended)
            }
            TaskOutcome::Failed { retryable: true, .. } => {
                self.scheduler.release_claim(&claim, now_ms)
            }
            TaskOutcome::Failed { retryable: false, .. } => {
                self.scheduler.finish_claim(&claim, now_ms, TaskState::Failed)
            }
            TaskOutcome::Canceled {} => {
                self.scheduler.finish_claim(&claim, now_ms, TaskState::Canceled)
            }
            TaskOutcome::TimedOut {} => {
                self.scheduler.finish_claim(&claim, now_ms, TaskState::TimedOut)
            }
        };
        match result {
            Ok(task) => {
                if task.state.is_terminal() {
                    let outcome = match task.state {
                        TaskState::TimedOut => TaskOutcome::TimedOut {},
                        TaskState::Canceled => TaskOutcome::Canceled {},
                        _ => outcome,
                    };
                    self.outcomes.insert(task_id, outcome);
                }
                Message::Committed { request_id, accepted: true }
            }
            Err(
                SchedulerError::LeaseLost
                | SchedulerError::Unknown(_)
                | SchedulerError::Full { .. },
            ) => Message::Committed { request_id, accepted: false },
            Err(error) => scheduler_error(Some(request_id), error),
        }
    }

    fn claim_from_token(&self, lease: &LeaseToken) -> Result<TaskClaim, ()> {
        let task_id = lease.task_id.parse().map_err(|_| ())?;
        let task = self.scheduler.get(task_id).cloned().ok_or(())?;
        Ok(TaskClaim {
            task,
            generation: lease.generation,
            lease_owner: lease.lease_owner.clone(),
            // The scheduler compares the server-side deadline, not a worker's
            // copy. TaskRPC tokens deliberately do not carry this mutable field.
            lease_until_ms: 0,
        })
    }
}

/// Per-connection TaskRPC identity state. A listener can keep one session per
/// socket while sharing a broker behind its own short-lived synchronization.
#[derive(Default)]
pub struct TaskRpcSession {
    worker_id: Option<String>,
}

impl TaskRpcSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn worker_id(&self) -> Option<&str> {
        self.worker_id.as_deref()
    }

    pub fn handle(
        &mut self,
        broker: &mut TaskRpcBroker,
        now_ms: u64,
        envelope: Envelope,
    ) -> Envelope {
        let request_id = request_id(&envelope.message);
        if let Some(owner) = self.worker_id.as_deref() {
            if worker_request_matches(&envelope.message, owner) {
                broker.handle(now_ms, envelope)
            } else {
                error_response(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "TaskRPC message does not belong to this worker connection".into(),
                )
            }
        } else if let Message::Hello { worker_id } = &envelope.message {
            if envelope.validate().is_ok() {
                self.worker_id = Some(worker_id.clone());
            }
            broker.handle(now_ms, envelope)
        } else {
            error_response(
                request_id,
                ErrorCode::InvalidRequest,
                "TaskRPC hello must be the first message".into(),
            )
        }
    }

    /// Requeue claims owned by this connection as queue capacity permits. Call
    /// this after clean EOF, truncated frames, protocol failure, or transport
    /// failure. Claims that cannot be requeued remain generation-fenced and are
    /// recovered after their lease expires.
    pub fn disconnect(
        &mut self,
        broker: &mut TaskRpcBroker,
        now_ms: u64,
    ) -> Result<Vec<TaskId>, SchedulerError> {
        let Some(worker_id) = self.worker_id.take() else {
            return Ok(Vec::new());
        };
        broker.disconnect_worker(&worker_id, now_ms)
    }
}

/// Cooperative shutdown handle for a Unix TaskRPC server and its connections.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct TaskRpcServerShutdown {
    sender: watch::Sender<bool>,
}

#[cfg(unix)]
impl Default for TaskRpcServerShutdown {
    fn default() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }
}

#[cfg(unix)]
impl TaskRpcServerShutdown {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }
}

/// Fatal listener failures. Invalid frames and ordinary connection I/O errors
/// are isolated to their connection and do not stop the server.
#[cfg(unix)]
#[derive(Debug, thiserror::Error)]
pub enum TaskRpcServerError {
    #[error("TaskRPC Unix listener: {0}")]
    Listener(#[source] std::io::Error),
    #[error("TaskRPC connection task panicked or was canceled: {0}")]
    ConnectionTask(#[from] tokio::task::JoinError),
}

/// A connected Unix TaskRPC worker. Requests are serialized through `&mut
/// self`, making response correlation explicit and preventing one worker from
/// accidentally sharing a connection concurrently.
#[cfg(unix)]
pub struct TaskRpcWorker {
    worker_id: String,
    stream: UnixStream,
    next_request_id: u64,
}

#[cfg(unix)]
#[derive(Debug, thiserror::Error)]
pub enum TaskRpcWorkerError {
    #[error(transparent)]
    Protocol(#[from] TaskRpcError),
    #[error("TaskRPC server rejected the request ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("TaskRPC response did not match request {request_id}: {message}")]
    UnexpectedResponse { request_id: u64, message: &'static str },
    #[error("TaskRPC request id space is exhausted")]
    RequestIdExhausted,
}

/// Executes one claimed module task at a time. Keeping one outstanding lease
/// prevents queued work from expiring while another handler consumes the
/// isolate's full request budget.
#[cfg(unix)]
pub struct TaskModuleWorker {
    rpc: TaskRpcWorker,
    source: Arc<str>,
    config: IsolateConfig,
    lease_ms: u64,
}

#[cfg(unix)]
#[derive(Debug, thiserror::Error)]
pub enum TaskModuleWorkerError {
    #[error(transparent)]
    Rpc(#[from] TaskRpcWorkerError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("task module inspection worker failed: {0}")]
    InspectionTask(#[from] tokio::task::JoinError),
    #[error("task lease must exceed the isolate timeout by at least 1000ms")]
    LeaseTooShort,
    #[error("task lease was lost before its outcome could be committed")]
    LeaseLost,
}

#[cfg(unix)]
impl TaskModuleWorker {
    pub async fn connect(
        socket_path: impl AsRef<Path>,
        worker_id: impl Into<String>,
        source: impl Into<String>,
        config: IsolateConfig,
        lease_ms: u64,
    ) -> Result<Self, TaskModuleWorkerError> {
        let minimum_lease = config.request_timeout_ms.checked_add(1_000);
        if minimum_lease.is_none_or(|minimum| lease_ms < minimum)
            || lease_ms > tysel_task_rpc::MAX_LEASE_MS
        {
            return Err(TaskModuleWorkerError::LeaseTooShort);
        }
        let source: Arc<str> = Arc::from(source.into());
        let inspection_source = Arc::clone(&source);
        tokio::task::spawn_blocking(move || inspect_task_module(&inspection_source, config))
            .await??;
        let rpc = TaskRpcWorker::connect(socket_path, worker_id).await?;
        Ok(Self { rpc, source, config, lease_ms })
    }

    pub fn worker_id(&self) -> &str {
        self.rpc.worker_id()
    }

    /// Claim and execute at most one task. Returns `false` when the scheduler
    /// currently has no runnable work.
    pub async fn run_once(&mut self) -> Result<bool, TaskModuleWorkerError> {
        let Some(lease) = self.rpc.claim(self.lease_ms, 1).await?.pop() else {
            return Ok(false);
        };
        let outcome = execute_module_lease(Arc::clone(&self.source), self.config, &lease).await;
        if !self.rpc.commit(lease.token, outcome).await? {
            return Err(TaskModuleWorkerError::LeaseLost);
        }
        Ok(true)
    }
}

#[cfg(unix)]
async fn execute_module_lease(
    source: Arc<str>,
    config: IsolateConfig,
    lease: &TaskLease,
) -> TaskOutcome {
    let Some(handler) = lease.task.trigger.handler_name().map(str::to_owned) else {
        return failed_outcome("HTTP tasks cannot run in a module task worker", false);
    };
    let input_json = match serde_json::to_string(&lease.task.input) {
        Ok(input) => input,
        Err(error) => return failed_outcome(&error.to_string(), false),
    };
    let request_id = lease.token.task_id.as_str().to_owned();
    let deadline_ms = lease
        .task
        .deadline_ms
        .unwrap_or_else(|| task_rpc_now_ms().saturating_add(config.request_timeout_ms.max(1)));
    let result = tokio::task::spawn_blocking(move || {
        invoke_task_module(&source, &handler, &input_json, &request_id, deadline_ms, config)
    })
    .await;
    match result {
        Ok(Ok(value)) => {
            let result = engine_value_to_json(value);
            match serde_json::to_vec(&result) {
                Ok(bytes) if bytes.len() <= tysel_task_rpc::MAX_RESULT_BYTES => {
                    TaskOutcome::Completed { result }
                }
                Ok(bytes) => failed_outcome(
                    &format!(
                        "task result is {} bytes; maximum is {}",
                        bytes.len(),
                        tysel_task_rpc::MAX_RESULT_BYTES
                    ),
                    false,
                ),
                Err(error) => failed_outcome(&error.to_string(), false),
            }
        }
        Ok(Err(EngineError::Interrupted(InterruptReason::Timeout))) => TaskOutcome::TimedOut {},
        Ok(Err(EngineError::Interrupted(InterruptReason::Cancelled))) => TaskOutcome::Canceled {},
        Ok(Err(EngineError::Suspended)) => TaskOutcome::Suspended {},
        Ok(Err(error)) => failed_outcome(&error.to_string(), false),
        Err(error) => failed_outcome(&error.to_string(), true),
    }
}

#[cfg(unix)]
fn failed_outcome(error: &str, retryable: bool) -> TaskOutcome {
    let mut error = error.to_owned();
    if error.len() > tysel_task_rpc::MAX_ERROR_BYTES {
        let mut end = tysel_task_rpc::MAX_ERROR_BYTES;
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        error.truncate(end);
    }
    TaskOutcome::Failed { error, retryable }
}

#[cfg(unix)]
fn engine_value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(value) => serde_json::Value::Bool(value),
        Value::Number(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(value) => serde_json::Value::String(value),
        Value::Bytes(bytes) => serde_json::Value::Array(
            bytes.into_iter().map(|byte| serde_json::Value::from(u64::from(byte))).collect(),
        ),
        Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(engine_value_to_json).collect())
        }
        Value::Record(fields) => serde_json::Value::Object(
            fields.into_iter().map(|(name, value)| (name, engine_value_to_json(value))).collect(),
        ),
    }
}

#[cfg(unix)]
impl TaskRpcWorker {
    pub async fn connect(
        socket_path: impl AsRef<Path>,
        worker_id: impl Into<String>,
    ) -> Result<Self, TaskRpcWorkerError> {
        let worker_id = worker_id.into();
        let stream = UnixStream::connect(socket_path).await.map_err(TaskRpcError::from)?;
        let mut worker = Self { worker_id: worker_id.clone(), stream, next_request_id: 1 };
        let response = worker.request(Message::Hello { worker_id }).await?;
        if !matches!(response, Message::Ready {}) {
            return Err(TaskRpcWorkerError::UnexpectedResponse {
                request_id: 0,
                message: "expected ready handshake",
            });
        }
        Ok(worker)
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    pub async fn claim(
        &mut self,
        lease_ms: u64,
        limit: u16,
    ) -> Result<Vec<TaskLease>, TaskRpcWorkerError> {
        let request_id = self.request_id()?;
        match self
            .request(Message::Claim {
                request_id,
                worker_id: self.worker_id.clone(),
                lease_ms,
                limit,
            })
            .await?
        {
            Message::Claimed { request_id: response_id, leases } if response_id == request_id => {
                Ok(leases)
            }
            _ => Err(TaskRpcWorkerError::UnexpectedResponse {
                request_id,
                message: "expected matching claimed response",
            }),
        }
    }

    pub async fn renew(
        &mut self,
        lease: LeaseToken,
        lease_ms: u64,
    ) -> Result<Option<TaskLease>, TaskRpcWorkerError> {
        let request_id = self.request_id()?;
        match self.request(Message::Renew { request_id, lease, lease_ms }).await? {
            Message::Renewed { request_id: response_id, lease } if response_id == request_id => {
                Ok(lease)
            }
            _ => Err(TaskRpcWorkerError::UnexpectedResponse {
                request_id,
                message: "expected matching renewed response",
            }),
        }
    }

    pub async fn release(&mut self, lease: LeaseToken) -> Result<bool, TaskRpcWorkerError> {
        let request_id = self.request_id()?;
        match self.request(Message::Release { request_id, lease }).await? {
            Message::Released { request_id: response_id, released }
                if response_id == request_id =>
            {
                Ok(released)
            }
            _ => Err(TaskRpcWorkerError::UnexpectedResponse {
                request_id,
                message: "expected matching released response",
            }),
        }
    }

    pub async fn commit(
        &mut self,
        lease: LeaseToken,
        outcome: TaskOutcome,
    ) -> Result<bool, TaskRpcWorkerError> {
        let request_id = self.request_id()?;
        match self.request(Message::Commit { request_id, lease, outcome }).await? {
            Message::Committed { request_id: response_id, accepted }
                if response_id == request_id =>
            {
                Ok(accepted)
            }
            _ => Err(TaskRpcWorkerError::UnexpectedResponse {
                request_id,
                message: "expected matching committed response",
            }),
        }
    }

    fn request_id(&mut self) -> Result<u64, TaskRpcWorkerError> {
        let request_id = self.next_request_id;
        self.next_request_id =
            request_id.checked_add(1).ok_or(TaskRpcWorkerError::RequestIdExhausted)?;
        Ok(request_id)
    }

    async fn request(&mut self, message: Message) -> Result<Message, TaskRpcWorkerError> {
        write_async_message(&mut self.stream, &Envelope::new(message)).await?;
        let response = read_async_message(&mut self.stream).await?.ok_or_else(|| {
            TaskRpcError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "TaskRPC server closed before responding",
            ))
        })?;
        match response.message {
            Message::Error { code, message, .. } => {
                Err(TaskRpcWorkerError::Rejected { code, message })
            }
            message => Ok(message),
        }
    }
}

/// Serve TaskRPC over an already-bound Unix-domain listener.
///
/// The caller retains ownership of socket-path creation and cleanup. Each
/// connection gets its own identity-bound [`TaskRpcSession`]; the shared broker
/// is locked only while one decoded request is dispatched or disconnected.
#[cfg(unix)]
pub async fn serve_task_rpc_unix(
    listener: UnixListener,
    broker: Arc<Mutex<TaskRpcBroker>>,
    shutdown: TaskRpcServerShutdown,
) -> Result<(), TaskRpcServerError> {
    let mut connections = JoinSet::new();
    let active_workers = Arc::new(Mutex::new(HashSet::new()));
    let server_result = loop {
        tokio::select! {
            () = shutdown.cancelled() => break Ok(()),
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        connections.spawn(serve_task_rpc_connection(
                            stream,
                            Arc::clone(&broker),
                            Arc::clone(&active_workers),
                            shutdown.clone(),
                        ));
                    }
                    Err(error) => break Err(TaskRpcServerError::Listener(error)),
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = joined {
                    break Err(TaskRpcServerError::ConnectionTask(error));
                }
            }
        }
    };

    shutdown.cancel();
    while let Some(joined) = connections.join_next().await {
        joined?;
    }
    server_result
}

#[cfg(unix)]
async fn serve_task_rpc_connection(
    stream: UnixStream,
    broker: Arc<Mutex<TaskRpcBroker>>,
    active_workers: Arc<Mutex<HashSet<String>>>,
    shutdown: TaskRpcServerShutdown,
) {
    let (mut reader, mut writer) = stream.into_split();
    let mut session = TaskRpcSession::new();

    loop {
        let request = tokio::select! {
            () = shutdown.cancelled() => break,
            request = read_async_message(&mut reader) => match request {
                Ok(Some(request)) => request,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(error = %error, "closing invalid TaskRPC connection");
                    break;
                }
            },
        };
        let registering = match (&request.message, session.worker_id()) {
            (Message::Hello { worker_id }, None) => Some(worker_id.clone()),
            _ => None,
        };
        let duplicate_worker = if let Some(worker_id) = registering {
            !active_workers.lock().await.insert(worker_id)
        } else {
            false
        };
        let response = if duplicate_worker {
            error_response(
                None,
                ErrorCode::InvalidRequest,
                "TaskRPC worker id is already connected".into(),
            )
        } else {
            let mut broker = broker.lock().await;
            session.handle(&mut broker, task_rpc_now_ms(), request)
        };
        if let Err(error) = write_async_message(&mut writer, &response).await {
            tracing::warn!(error = %error, "closing failed TaskRPC connection");
            break;
        }
    }

    let worker_id = session.worker_id().map(str::to_owned);
    let disconnected = {
        let mut broker = broker.lock().await;
        session.disconnect(&mut broker, task_rpc_now_ms())
    };
    if let Some(worker_id) = worker_id {
        active_workers.lock().await.remove(&worker_id);
    }
    if let Err(error) = disconnected {
        tracing::warn!(error = %error, "failed to requeue disconnected TaskRPC worker");
    }
}

#[cfg(unix)]
async fn read_async_message(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Option<Envelope>, TaskRpcError> {
    let mut length = [0; 4];
    let read = loop {
        match reader.read(&mut length[..1]).await {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => break result?,
        }
    };
    if read == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length[1..]).await?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_TASK_RPC_FRAME {
        return Err(TaskRpcError::FrameTooLarge(length));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    Ok(Some(decode_message(&bytes)?))
}

#[cfg(unix)]
async fn write_async_message(
    writer: &mut (impl AsyncWrite + Unpin),
    envelope: &Envelope,
) -> Result<(), TaskRpcError> {
    let bytes = encode_message(envelope)?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(unix)]
fn task_rpc_now_ms() -> u64 {
    let elapsed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn wire_lease(claim: &TaskClaim) -> Result<TaskLease, TaskRpcError> {
    Ok(TaskLease {
        token: LeaseToken {
            task_id: WireTaskId::new(claim.task.meta.id),
            generation: claim.generation,
            lease_owner: claim.lease_owner.clone(),
        },
        task: TaskDescriptor::from_task(&claim.task)?,
        lease_until_ms: claim.lease_until_ms,
    })
}

fn preview_wire_lease(task: &Task, lease_owner: &str) -> Result<TaskLease, TaskRpcError> {
    Ok(TaskLease {
        token: LeaseToken {
            task_id: WireTaskId::new(task.meta.id),
            generation: u64::MAX,
            lease_owner: lease_owner.into(),
        },
        task: TaskDescriptor::from_task(task)?,
        lease_until_ms: u64::MAX,
    })
}

fn request_id(message: &Message) -> Option<u64> {
    match message {
        Message::Hello { .. } | Message::Ready {} => None,
        Message::Claim { request_id, .. }
        | Message::Claimed { request_id, .. }
        | Message::Renew { request_id, .. }
        | Message::Renewed { request_id, .. }
        | Message::Release { request_id, .. }
        | Message::Released { request_id, .. }
        | Message::Cancel { request_id, .. }
        | Message::Canceled { request_id, .. }
        | Message::Commit { request_id, .. }
        | Message::Committed { request_id, .. } => Some(*request_id),
        Message::Error { request_id, .. } => *request_id,
    }
}

fn worker_request_matches(message: &Message, worker_id: &str) -> bool {
    match message {
        Message::Claim { worker_id: requested, .. } => requested == worker_id,
        Message::Renew { lease, .. }
        | Message::Release { lease, .. }
        | Message::Commit { lease, .. } => lease.lease_owner == worker_id,
        Message::Cancel { .. } => false,
        Message::Hello { .. }
        | Message::Ready {}
        | Message::Claimed { .. }
        | Message::Renewed { .. }
        | Message::Released { .. }
        | Message::Canceled { .. }
        | Message::Committed { .. }
        | Message::Error { .. } => false,
    }
}

fn scheduler_error(request_id: Option<u64>, error: SchedulerError) -> Message {
    let code = match error {
        SchedulerError::Unknown(_) => ErrorCode::TaskNotFound,
        SchedulerError::LeaseLost => ErrorCode::LeaseLost,
        SchedulerError::InvalidLeaseOwner
        | SchedulerError::InvalidLeaseDuration
        | SchedulerError::InvalidClaimOutcome(_) => ErrorCode::InvalidRequest,
        _ => ErrorCode::Internal,
    };
    error_message(request_id, code, &error.to_string())
}

fn error_response(request_id: Option<u64>, code: ErrorCode, message: String) -> Envelope {
    Envelope::new(error_message(request_id, code, &message))
}

fn error_message(request_id: Option<u64>, code: ErrorCode, message: &str) -> Message {
    Message::Error { request_id, code, message: message.into() }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    #[cfg(unix)]
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use super::*;
    #[cfg(unix)]
    use tokio::{net::UnixStream, sync::Mutex};
    use tysel_task::{TaskMeta, TaskTrigger};
    use tysel_task_rpc::{WireTaskTrigger, read_message_opt};

    fn task(id: u128) -> Task {
        task_with_deadline(id, None)
    }

    fn task_with_deadline(id: u128, deadline_ms: Option<u64>) -> Task {
        Task::new(
            TaskMeta {
                id: TaskId(id),
                application_id: "test".into(),
                tenant_id: None,
                idempotency_key: None,
                trace_id: None,
            },
            TaskTrigger::Agent { name: format!("agent-{id}") },
            deadline_ms,
        )
    }

    fn claim(broker: &mut TaskRpcBroker, now_ms: u64, worker: &str, request_id: u64) -> TaskLease {
        claim_for(broker, now_ms, worker, request_id, 10)
    }

    fn claim_for(
        broker: &mut TaskRpcBroker,
        now_ms: u64,
        worker: &str,
        request_id: u64,
        lease_ms: u64,
    ) -> TaskLease {
        let response = broker.handle(
            now_ms,
            Envelope::new(Message::Claim {
                request_id,
                worker_id: worker.into(),
                lease_ms,
                limit: 1,
            }),
        );
        let Message::Claimed { mut leases, .. } = response.message else {
            panic!("expected claim response")
        };
        leases.pop().expect("one claimed task")
    }

    #[test]
    fn handshake_claim_renew_and_commit_complete_a_task() {
        let mut broker = TaskRpcBroker::new(2).unwrap();
        broker.enqueue(task(1)).unwrap();
        assert_eq!(
            broker.handle(0, Envelope::new(Message::Hello { worker_id: "worker-a".into() })),
            Envelope::new(Message::Ready {})
        );
        let lease = claim(&mut broker, 0, "worker-a", 1);
        let renewed = broker.handle(
            5,
            Envelope::new(Message::Renew {
                request_id: 2,
                lease: lease.token.clone(),
                lease_ms: 20,
            }),
        );
        let Message::Renewed { lease: Some(renewed), .. } = renewed.message else {
            panic!("expected renewed lease")
        };
        assert_eq!(renewed.lease_until_ms, 25);
        let outcome = TaskOutcome::Completed { result: serde_json::json!({"ok": true}) };
        let response = broker.handle(
            10,
            Envelope::new(Message::Commit {
                request_id: 3,
                lease: renewed.token,
                outcome: outcome.clone(),
            }),
        );
        assert_eq!(response.message, Message::Committed { request_id: 3, accepted: true });
        assert_eq!(broker.task(TaskId(1)).unwrap().state, TaskState::Completed);
        assert_eq!(broker.outcome(TaskId(1)), Some(&outcome));
    }

    #[test]
    fn crash_recovery_fences_a_late_commit() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(2)).unwrap();
        let stale = claim(&mut broker, 0, "worker-a", 1);
        assert_eq!(broker.requeue_expired(10, 1).unwrap(), vec![TaskId(2)]);
        let current = claim(&mut broker, 10, "worker-b", 2);
        assert_eq!(current.token.generation, stale.token.generation + 1);

        let late = broker.handle(
            11,
            Envelope::new(Message::Commit {
                request_id: 3,
                lease: stale.token,
                outcome: TaskOutcome::Completed { result: serde_json::Value::Null },
            }),
        );
        assert_eq!(late.message, Message::Committed { request_id: 3, accepted: false });
        let accepted = broker.handle(
            11,
            Envelope::new(Message::Commit {
                request_id: 4,
                lease: current.token,
                outcome: TaskOutcome::Completed { result: serde_json::Value::Null },
            }),
        );
        assert_eq!(accepted.message, Message::Committed { request_id: 4, accepted: true });
    }

    #[test]
    fn retryable_failure_requeues_with_a_new_generation() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(3)).unwrap();
        let first = claim(&mut broker, 0, "worker-a", 1);
        let response = broker.handle(
            1,
            Envelope::new(Message::Commit {
                request_id: 2,
                lease: first.token.clone(),
                outcome: TaskOutcome::Failed { error: "retry".into(), retryable: true },
            }),
        );
        assert_eq!(response.message, Message::Committed { request_id: 2, accepted: true });
        assert_eq!(broker.task(TaskId(3)).unwrap().state, TaskState::Queued);
        assert!(broker.outcome(TaskId(3)).is_none());
        let retried = claim(&mut broker, 1, "worker-b", 3);
        assert_eq!(retried.token.generation, first.token.generation + 1);
    }

    #[test]
    fn retryable_failure_after_deadline_records_timeout() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task_with_deadline(5, Some(10))).unwrap();
        let lease = claim_for(&mut broker, 0, "worker-a", 1, 20);
        let response = broker.handle(
            10,
            Envelope::new(Message::Commit {
                request_id: 2,
                lease: lease.token,
                outcome: TaskOutcome::Failed { error: "too late".into(), retryable: true },
            }),
        );
        assert_eq!(response.message, Message::Committed { request_id: 2, accepted: true });
        assert_eq!(broker.task(TaskId(5)).unwrap().state, TaskState::TimedOut);
        assert_eq!(broker.outcome(TaskId(5)), Some(&TaskOutcome::TimedOut {}));
    }

    #[test]
    fn enqueue_rejects_tasks_that_cannot_fit_the_wire_contract() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        let mut invalid = task(50);
        invalid.trigger =
            TaskTrigger::Agent { name: "x".repeat(tysel_task_rpc::MAX_TASK_TRIGGER_BYTES + 1) };
        assert!(matches!(
            broker.enqueue(invalid),
            Err(TaskRpcBrokerError::Protocol(TaskRpcError::TaskTriggerTooLarge))
        ));
        assert!(broker.task(TaskId(50)).is_none());

        let oversized_input = task(51).with_input(serde_json::Value::String(
            "x".repeat(tysel_task_rpc::MAX_TASK_INPUT_BYTES),
        ));
        assert!(matches!(
            broker.enqueue(oversized_input),
            Err(TaskRpcBrokerError::Protocol(TaskRpcError::TaskInputTooLarge(_)))
        ));
        assert!(broker.task(TaskId(51)).is_none());
    }

    #[test]
    fn claim_splits_large_inputs_before_the_frame_limit() {
        let mut broker = TaskRpcBroker::new(2).unwrap();
        let input = serde_json::Value::String("x".repeat(tysel_task_rpc::MAX_TASK_INPUT_BYTES - 2));
        broker.enqueue(task(52).with_input(input.clone())).unwrap();
        broker.enqueue(task(53).with_input(input)).unwrap();

        let first = broker.handle(
            0,
            Envelope::new(Message::Claim {
                request_id: 1,
                worker_id: "worker-a".into(),
                lease_ms: 1_000,
                limit: 2,
            }),
        );
        let Message::Claimed { leases, .. } = &first.message else {
            panic!("expected claim response")
        };
        assert_eq!(leases.len(), 1);
        encode_message(&first).unwrap();

        let second = broker.handle(
            0,
            Envelope::new(Message::Claim {
                request_id: 2,
                worker_id: "worker-a".into(),
                lease_ms: 1_000,
                limit: 2,
            }),
        );
        let Message::Claimed { leases, .. } = &second.message else {
            panic!("expected claim response")
        };
        assert_eq!(leases.len(), 1);
        encode_message(&second).unwrap();
    }

    #[test]
    fn cancellation_fences_the_worker_and_unknown_tasks_are_negative_acks() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(4)).unwrap();
        let lease = claim(&mut broker, 0, "worker-a", 1);
        let canceled = broker.handle(
            1,
            Envelope::new(Message::Cancel { request_id: 2, task_id: WireTaskId::new(TaskId(4)) }),
        );
        assert_eq!(canceled.message, Message::Canceled { request_id: 2, canceled: true });
        let late = broker.handle(
            1,
            Envelope::new(Message::Commit {
                request_id: 3,
                lease: lease.token,
                outcome: TaskOutcome::Completed { result: serde_json::Value::Null },
            }),
        );
        assert_eq!(late.message, Message::Committed { request_id: 3, accepted: false });
        let unknown = broker.handle(
            1,
            Envelope::new(Message::Cancel { request_id: 4, task_id: WireTaskId::new(TaskId(999)) }),
        );
        assert_eq!(unknown.message, Message::Canceled { request_id: 4, canceled: false });
    }

    #[test]
    fn invalid_direction_and_version_return_protocol_errors() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        let response = broker.handle(0, Envelope::new(Message::Ready {}));
        assert!(matches!(response.message, Message::Error { code: ErrorCode::InvalidRequest, .. }));
        let response = broker.handle(
            0,
            Envelope { version: 2, message: Message::Hello { worker_id: "worker-a".into() } },
        );
        assert!(matches!(
            response.message,
            Message::Error { code: ErrorCode::VersionMismatch, .. }
        ));
    }

    #[test]
    fn framed_request_runs_through_the_broker() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(6)).unwrap();
        let request = Envelope::new(Message::Claim {
            request_id: 7,
            worker_id: "worker-a".into(),
            lease_ms: 10,
            limit: 1,
        });
        let mut encoded_request = Vec::new();
        write_message(&mut encoded_request, &request).unwrap();
        let mut encoded_response = Vec::new();
        broker.handle_frame(0, &mut Cursor::new(encoded_request), &mut encoded_response).unwrap();
        let response = read_message(&mut Cursor::new(encoded_response)).unwrap();
        assert!(matches!(
            response.message,
            Message::Claimed { request_id: 7, leases } if leases.len() == 1
        ));
    }

    #[test]
    fn session_disconnect_requeues_claims_and_fences_its_tokens() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(7)).unwrap();
        let mut input = Vec::new();
        write_message(&mut input, &Envelope::new(Message::Hello { worker_id: "worker-a".into() }))
            .unwrap();
        write_message(
            &mut input,
            &Envelope::new(Message::Claim {
                request_id: 1,
                worker_id: "worker-a".into(),
                lease_ms: 1_000,
                limit: 1,
            }),
        )
        .unwrap();
        let mut output = Vec::new();
        let mut session = TaskRpcSession::new();
        let mut input = Cursor::new(input);
        while let Some(request) = read_message_opt(&mut input).unwrap() {
            write_message(&mut output, &session.handle(&mut broker, 0, request)).unwrap();
        }
        assert_eq!(session.disconnect(&mut broker, 0).unwrap(), vec![TaskId(7)]);

        let mut output = Cursor::new(output);
        assert_eq!(read_message(&mut output).unwrap().message, Message::Ready {});
        let Message::Claimed { mut leases, .. } = read_message(&mut output).unwrap().message else {
            panic!("expected claimed response")
        };
        let stale = leases.pop().unwrap();
        assert_eq!(read_message_opt(&mut output).unwrap(), None);

        let current = claim(&mut broker, 0, "worker-b", 2);
        assert_eq!(current.token.generation, stale.token.generation + 1);
        let late = broker.handle(
            1,
            Envelope::new(Message::Commit {
                request_id: 3,
                lease: stale.token,
                outcome: TaskOutcome::Completed { result: serde_json::Value::Null },
            }),
        );
        assert_eq!(late.message, Message::Committed { request_id: 3, accepted: false });
    }

    #[test]
    fn session_requires_handshake_identity() {
        let mut broker = TaskRpcBroker::new(1).unwrap();
        broker.enqueue(task(8)).unwrap();
        let mut input = Vec::new();
        write_message(
            &mut input,
            &Envelope::new(Message::Claim {
                request_id: 1,
                worker_id: "worker-a".into(),
                lease_ms: 10,
                limit: 1,
            }),
        )
        .unwrap();
        write_message(&mut input, &Envelope::new(Message::Hello { worker_id: "worker-a".into() }))
            .unwrap();
        write_message(
            &mut input,
            &Envelope::new(Message::Claim {
                request_id: 2,
                worker_id: "worker-b".into(),
                lease_ms: 10,
                limit: 1,
            }),
        )
        .unwrap();
        let mut output = Vec::new();
        let mut session = TaskRpcSession::new();
        let mut input = Cursor::new(input);
        while let Some(request) = read_message_opt(&mut input).unwrap() {
            write_message(&mut output, &session.handle(&mut broker, 0, request)).unwrap();
        }
        assert!(session.disconnect(&mut broker, 0).unwrap().is_empty());

        let mut output = Cursor::new(output);
        assert!(matches!(
            read_message(&mut output).unwrap().message,
            Message::Error { code: ErrorCode::InvalidRequest, .. }
        ));
        assert_eq!(read_message(&mut output).unwrap().message, Message::Ready {});
        assert!(matches!(
            read_message(&mut output).unwrap().message,
            Message::Error { code: ErrorCode::InvalidRequest, .. }
        ));
        assert_eq!(broker.task(TaskId(8)).unwrap().state, TaskState::Queued);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_server_isolates_bad_clients_and_requeues_disconnects() {
        let socket_path = unique_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let broker = Arc::new(Mutex::new(TaskRpcBroker::new(1).unwrap()));
        broker
            .lock()
            .await
            .enqueue(task(9).with_input(serde_json::json!({"job": "nine"})))
            .unwrap();
        let shutdown = TaskRpcServerShutdown::new();
        let server =
            tokio::spawn(serve_task_rpc_unix(listener, Arc::clone(&broker), shutdown.clone()));

        // A truncated header closes only this connection, not the listener.
        let mut invalid = UnixStream::connect(&socket_path).await.unwrap();
        invalid.write_all(&[1, 0]).await.unwrap();
        drop(invalid);

        let mut worker_a = TaskRpcWorker::connect(&socket_path, "worker-a").await.unwrap();
        assert_eq!(worker_a.worker_id(), "worker-a");
        let stale = worker_a.claim(60_000, 1).await.unwrap().pop().expect("one claimed task");
        assert_eq!(stale.task.trigger, WireTaskTrigger::Agent { name: "agent-9".into() });
        assert_eq!(stale.task.input, serde_json::json!({"job": "nine"}));
        assert_eq!(stale.task.attempt, 1);
        drop(worker_a);

        wait_for_task_state(&broker, TaskId(9), TaskState::Queued).await;

        let mut worker_b = TaskRpcWorker::connect(&socket_path, "worker-b").await.unwrap();
        let current = worker_b.claim(60_000, 1).await.unwrap().pop().expect("requeued task");
        assert_eq!(current.token.generation, stale.token.generation + 1);

        assert!(matches!(
            TaskRpcWorker::connect(&socket_path, "worker-b").await,
            Err(TaskRpcWorkerError::Rejected { code: ErrorCode::InvalidRequest, .. })
        ));

        let late = broker.lock().await.handle(
            task_rpc_now_ms(),
            Envelope::new(Message::Commit {
                request_id: 3,
                lease: stale.token,
                outcome: TaskOutcome::Completed { result: serde_json::Value::Null },
            }),
        );
        assert_eq!(late.message, Message::Committed { request_id: 3, accepted: false });

        let renewed = worker_b
            .renew(current.token.clone(), 60_000)
            .await
            .unwrap()
            .expect("live lease renewal");
        assert!(worker_b.release(renewed.token).await.unwrap());
        let final_lease = worker_b.claim(60_000, 1).await.unwrap().pop().expect("released task");
        assert!(
            worker_b
                .commit(
                    final_lease.token,
                    TaskOutcome::Completed { result: serde_json::json!({"ok": true}) },
                )
                .await
                .unwrap()
        );
        assert_eq!(broker.lock().await.task(TaskId(9)).unwrap().state, TaskState::Completed);

        broker.lock().await.enqueue(task(10)).unwrap();
        let shutdown_lease = worker_b.claim(60_000, 1).await.unwrap().pop().expect("shutdown task");
        assert_eq!(shutdown_lease.task.trigger, WireTaskTrigger::Agent { name: "agent-10".into() });

        // Shutdown reaches an idle connection and runs its disconnect cleanup.
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TaskRPC server shutdown timed out")
            .expect("TaskRPC server task panicked")
            .expect("TaskRPC server failed");
        wait_for_task_state(&broker, TaskId(10), TaskState::Queued).await;
        drop(worker_b);
        std::fs::remove_file(socket_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn module_worker_executes_queue_input_and_commits_result() {
        let socket_path = unique_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let broker = Arc::new(Mutex::new(TaskRpcBroker::new(1).unwrap()));
        let task = Task::new(
            TaskMeta {
                id: TaskId(60),
                application_id: "test".into(),
                tenant_id: None,
                idempotency_key: None,
                trace_id: None,
            },
            TaskTrigger::Queue {
                name: "orders.created".into(),
                handler: "orders".into(),
                message_id: Some("message-60".into()),
            },
            Some(task_rpc_now_ms() + 10_000),
        )
        .with_input(serde_json::json!({"value": "ready"}));
        broker.lock().await.enqueue(task).unwrap();
        let shutdown = TaskRpcServerShutdown::new();
        let server =
            tokio::spawn(serve_task_rpc_unix(listener, Arc::clone(&broker), shutdown.clone()));

        let source = r#"
export default {
  tasks: {
    orders: {
      kind: "queue",
      name: "orders.created",
      async handler(message, ctx) {
        return { value: await tysel.echo(message.value), requestId: ctx.requestId };
      },
    },
  },
};
"#;
        let config = IsolateConfig {
            memory_limit_bytes: 32 * 1024 * 1024,
            cpu_ms_per_turn: 50,
            request_timeout_ms: 1_000,
        };
        let mut worker =
            TaskModuleWorker::connect(&socket_path, "module-worker", source, config, 2_000)
                .await
                .unwrap();
        assert_eq!(worker.worker_id(), "module-worker");
        assert!(worker.run_once().await.unwrap());
        assert!(!worker.run_once().await.unwrap());
        assert_eq!(
            broker.lock().await.outcome(TaskId(60)),
            Some(&TaskOutcome::Completed {
                result: serde_json::json!({
                    "value": "ready",
                    "requestId": "0000000000000000000000000000003c",
                }),
            })
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TaskRPC server shutdown timed out")
            .expect("TaskRPC server task panicked")
            .expect("TaskRPC server failed");
        drop(worker);
        std::fs::remove_file(socket_path).unwrap();
    }

    #[cfg(unix)]
    async fn wait_for_task_state(
        broker: &Arc<Mutex<TaskRpcBroker>>,
        task_id: TaskId,
        expected: TaskState,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = broker.lock().await.task(task_id).map(|task| task.state);
                if state == Some(expected) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("task state did not converge");
    }

    #[cfg(unix)]
    fn unique_socket_path() -> PathBuf {
        let nonce = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("tysel-task-rpc-{}-{nonce}.sock", std::process::id()))
    }
}
