use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use tysel_engine::Value;
use tysel_ipc::{IpcError, Message, WireValue, read_message, write_message};

use crate::broker::Broker;

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
    #[error("worker binary not found")]
    MissingWorker,
}

#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub memory_limit_bytes: usize,
    pub cpu_ms_per_turn: u64,
    pub request_timeout_ms: u64,
    pub rlimit_as_bytes: usize,
}

impl Default for WorkerSpec {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 8 * 1024 * 1024,
            cpu_ms_per_turn: 200,
            request_timeout_ms: 2_000,
            rlimit_as_bytes: 256 * 1024 * 1024,
        }
    }
}

pub struct Supervisor {
    worker_bin: PathBuf,
    spec: WorkerSpec,
    broker: Broker,
    next_id: u64,
    child: Option<WorkerConn>,
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
        let mut supervisor =
            Self { worker_bin, spec, broker: Broker::new(secrets), next_id: 1, child: None };
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
                Ok(())
            }
        }
    }

    pub fn kill_worker(&mut self) -> Result<(), IsolateError> {
        if let Some(mut conn) = self.child.take() {
            let _ = conn.child.kill();
            let _ = conn.child.wait();
        }
        Ok(())
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
        self.child = Some(conn);
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
    }
}
