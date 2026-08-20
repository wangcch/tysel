use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use tysel_engine::{HttpHead, HttpRequest, IsolateConfig, Value};
use tysel_ipc::{IpcError, Message, WireValue, read_message, write_message};

use crate::broker::Broker;

/// Request/response bodies larger than this cannot fit in a 64KiB IPC frame
/// with headers and JSON envelope.
pub const MAX_ISOLATED_HTTP_BODY: usize = 32 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IsolateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error("worker: {0}")]
    Worker(String),
    #[error("broker: {0}")]
    Broker(String),
    #[error("resource limit: {0}")]
    Limit(String),
    #[error(
        "worker binary not found; set TYSEL_WORKER or place tysel-worker next to the tysel binary"
    )]
    MissingWorker,
}

#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub memory_limit_bytes: usize,
    pub cpu_ms_per_turn: u64,
    pub request_timeout_ms: u64,
    pub rlimit_as_bytes: usize,
    pub app: String,
    pub json_logs: bool,
}

impl Default for WorkerSpec {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 8 * 1024 * 1024,
            cpu_ms_per_turn: 200,
            request_timeout_ms: 2_000,
            rlimit_as_bytes: 256 * 1024 * 1024,
            app: String::new(),
            json_logs: false,
        }
    }
}

impl From<IsolateConfig> for WorkerSpec {
    fn from(config: IsolateConfig) -> Self {
        Self {
            memory_limit_bytes: config.memory_limit_bytes,
            cpu_ms_per_turn: config.cpu_ms_per_turn,
            request_timeout_ms: config.request_timeout_ms,
            ..Self::default()
        }
    }
}

/// Resolve `tysel-worker` for isolated HTTP. `TYSEL_WORKER` wins; otherwise a
/// sibling of the current executable is used.
pub fn locate_worker() -> Result<PathBuf, IsolateError> {
    if let Some(path) = std::env::var_os("TYSEL_WORKER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(IsolateError::MissingWorker);
    }
    let exe = std::env::current_exe().map_err(IsolateError::from)?;
    let dir = exe.parent().ok_or(IsolateError::MissingWorker)?;
    let mut sibling = dir.join("tysel-worker");
    if cfg!(windows) {
        sibling.set_extension("exe");
    }
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(IsolateError::MissingWorker)
}

pub struct Supervisor {
    worker_bin: PathBuf,
    spec: WorkerSpec,
    broker: Broker,
    next_id: u64,
    cgroup: Option<crate::cgroup::Guard>,
    child: Option<WorkerConn>,
    handler_source: Option<String>,
    handler_secret_names: Vec<String>,
}

struct WorkerConn {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Supervisor {
    pub fn spawn(
        worker_bin: impl AsRef<Path>,
        spec: WorkerSpec,
        secrets: HashMap<String, String>,
    ) -> Result<Self, IsolateError> {
        let worker_bin = worker_bin.as_ref().to_path_buf();
        if !worker_bin.is_file() {
            return Err(IsolateError::MissingWorker);
        }
        let mut supervisor = Self {
            worker_bin,
            spec,
            broker: Broker::new(secrets),
            next_id: 1,
            cgroup: None,
            child: None,
            handler_source: None,
            handler_secret_names: Vec::new(),
        };
        supervisor.ensure_worker()?;
        Ok(supervisor)
    }

    pub fn eval(&mut self, source: &str) -> Result<Value, IsolateError> {
        match self.eval_inner(source) {
            Ok(value) => Ok(value),
            Err(err) if self.worker_exited() => {
                self.ensure_worker()?;
                self.eval_inner(source)
                    .map_err(|retry| IsolateError::Worker(format!("{err}; retry: {retry}")))
            }
            Err(err) => Err(err),
        }
    }

    pub fn overalloc(&mut self) -> Result<(), IsolateError> {
        self.ensure_worker()?;
        let conn = self.child.as_mut().expect("worker");
        write_message(&mut conn.stdin, &Message::Overalloc)?;
        match read_message(&mut conn.stdout) {
            Ok(other) => Err(IsolateError::Worker(format!("worker survived overalloc: {other:?}"))),
            Err(_) => {
                let _ = conn.child.wait();
                self.child = None;
                self.cgroup = None;
                Ok(())
            }
        }
    }

    pub fn kill_worker(&mut self) -> Result<(), IsolateError> {
        if let Some(mut conn) = self.child.take() {
            let _ = conn.child.kill();
            let _ = conn.child.wait();
        }
        self.cgroup = None;
        Ok(())
    }

    pub fn load_handler(
        &mut self,
        source: &str,
        secret_names: Vec<String>,
    ) -> Result<(), IsolateError> {
        self.handler_source = Some(source.to_owned());
        self.handler_secret_names = secret_names;
        self.ensure_worker()?;
        self.send_load()
    }

    pub fn http(&mut self, request: &HttpRequest) -> Result<(HttpHead, Vec<u8>), IsolateError> {
        match self.http_inner(request) {
            Ok(value) => Ok(value),
            Err(err) if self.worker_exited() => self
                .http_inner(request)
                .map_err(|retry| IsolateError::Worker(format!("{err}; retry: {retry}"))),
            Err(err) => Err(err),
        }
    }

    fn send_load(&mut self) -> Result<(), IsolateError> {
        let source = self
            .handler_source
            .clone()
            .ok_or_else(|| IsolateError::Worker("handler source missing".into()))?;
        let secret_names = self.handler_secret_names.clone();
        let conn = self.child.as_mut().expect("worker");
        write_message(&mut conn.stdin, &Message::Load { source, secret_names })?;
        match read_message(&mut conn.stdout)? {
            Message::Loaded => Ok(()),
            Message::LoadErr { error } => Err(IsolateError::Worker(error)),
            other => Err(IsolateError::Worker(format!("expected loaded, got {other:?}"))),
        }
    }

    fn http_inner(&mut self, request: &HttpRequest) -> Result<(HttpHead, Vec<u8>), IsolateError> {
        if request.body.len() > MAX_ISOLATED_HTTP_BODY {
            return Err(IsolateError::Worker("isolated request body exceeds 32KiB IPC cap".into()));
        }
        self.ensure_worker()?;
        let id = self.next_id;
        self.next_id += 1;
        {
            let conn = self.child.as_mut().expect("worker");
            write_message(
                &mut conn.stdin,
                &Message::Http {
                    id,
                    method: request.method.clone(),
                    url: request.url.clone(),
                    headers: request.headers.clone(),
                    body: String::from_utf8_lossy(&request.body).into_owned(),
                    request_id: request.request_id,
                },
            )?;
        }
        loop {
            let message = {
                let conn = self.child.as_mut().expect("worker");
                read_message(&mut conn.stdout)?
            };
            match message {
                Message::CapCall { id: cap_id, op, args } => {
                    let reply = match self.broker.call(&op, &args) {
                        Ok(value) => Message::CapOk { id: cap_id, value: WireValue::from(value) },
                        Err(err) => Message::CapErr { id: cap_id, error: err.to_string() },
                    };
                    let conn = self.child.as_mut().expect("worker");
                    write_message(&mut conn.stdin, &reply)?;
                }
                Message::HttpOk { id: reply_id, status, headers, body, websocket }
                    if reply_id == id =>
                {
                    if websocket {
                        return Err(IsolateError::Worker(
                            "websocket is not available in the isolated profile".into(),
                        ));
                    }
                    if body.len() > MAX_ISOLATED_HTTP_BODY {
                        return Err(IsolateError::Worker(
                            "isolated response body exceeds 32KiB IPC cap".into(),
                        ));
                    }
                    return Ok((HttpHead { status, headers, websocket: false }, body.into_bytes()));
                }
                Message::HttpErr { id: reply_id, error } if reply_id == id => {
                    return Err(IsolateError::Worker(error));
                }
                other => return Err(IsolateError::Worker(format!("unexpected message {other:?}"))),
            }
        }
    }

    fn eval_inner(&mut self, source: &str) -> Result<Value, IsolateError> {
        self.ensure_worker()?;
        let id = self.next_id;
        self.next_id += 1;
        {
            let conn = self.child.as_mut().expect("worker");
            write_message(&mut conn.stdin, &Message::Eval { id, source: source.to_owned() })?;
        }
        loop {
            let message = {
                let conn = self.child.as_mut().expect("worker");
                read_message(&mut conn.stdout)?
            };
            match message {
                Message::CapCall { id: cap_id, op, args } => {
                    let reply = match self.broker.call(&op, &args) {
                        Ok(value) => Message::CapOk { id: cap_id, value: WireValue::from(value) },
                        Err(err) => Message::CapErr { id: cap_id, error: err.to_string() },
                    };
                    let conn = self.child.as_mut().expect("worker");
                    write_message(&mut conn.stdin, &reply)?;
                }
                Message::EvalOk { id: eval_id, value } if eval_id == id => {
                    return Ok(Value::from(value));
                }
                Message::EvalErr { id: eval_id, error } if eval_id == id => {
                    return Err(IsolateError::Worker(error));
                }
                other => return Err(IsolateError::Worker(format!("unexpected message {other:?}"))),
            }
        }
    }

    fn ensure_worker(&mut self) -> Result<(), IsolateError> {
        if self.child.is_some() && !self.worker_exited() {
            return Ok(());
        }
        self.child = None;
        self.cgroup = None;
        let mut command = Command::new(&self.worker_bin);
        command.env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = command.spawn()?;
        let stdin =
            child.stdin.take().ok_or_else(|| IsolateError::Worker("missing stdin".into()))?;
        let stdout =
            child.stdout.take().ok_or_else(|| IsolateError::Worker("missing stdout".into()))?;
        let mut conn = WorkerConn { child, stdin, stdout: BufReader::new(stdout) };
        match read_message(&mut conn.stdout) {
            Ok(Message::WorkerReady) => {}
            Ok(other) => {
                return Err(IsolateError::Worker(format!("expected ready, got {other:?}")));
            }
            Err(err) => {
                let _ = conn.child.wait();
                return Err(IsolateError::Worker(format!("worker failed to start: {err}")));
            }
        }
        write_message(
            &mut conn.stdin,
            &Message::Start {
                memory_limit_bytes: self.spec.memory_limit_bytes,
                cpu_ms_per_turn: self.spec.cpu_ms_per_turn,
                request_timeout_ms: self.spec.request_timeout_ms,
                rlimit_as_bytes: self.spec.rlimit_as_bytes,
                app: self.spec.app.clone(),
                json_logs: self.spec.json_logs,
            },
        )?;
        match read_message(&mut conn.stdout) {
            Ok(Message::Started) => {}
            Ok(other) => {
                return Err(IsolateError::Worker(format!("expected started, got {other:?}")));
            }
            Err(err) => return Err(IsolateError::Worker(format!("start handshake failed: {err}"))),
        }
        let _ = conn.stdin.flush();
        let pid = conn.child.id();
        self.cgroup = crate::cgroup::attach(pid, self.spec.rlimit_as_bytes);
        self.child = Some(conn);
        if self.handler_source.is_some() {
            self.send_load()?;
        }
        Ok(())
    }

    fn worker_exited(&mut self) -> bool {
        let Some(conn) = self.child.as_mut() else {
            return true;
        };
        match conn.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Some(conn) = self.child.as_mut() {
            let _ = write_message(&mut conn.stdin, &Message::Shutdown);
            let _ = conn.child.wait();
        }
        self.child = None;
        self.cgroup = None;
    }
}
