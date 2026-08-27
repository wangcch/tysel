use std::collections::HashMap;
use std::io::{self, BufReader, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use tysel_engine::{HttpRequest, InterruptReason, IsolateConfig, Value};
use tysel_engine_qjs::{
    IoCompletion, IoRequest, IsolateCancel, IsolatePool, OpId, eval_with_reactor_deadline,
    inspect_task_module, invoke_task_module, open_bridge,
};
use tysel_ipc::{Message, TaskErrorKind, WireValue, read_message, write_message};
use tysel_policy::Policy;

use crate::landlock;
use crate::rlimit;
use crate::seccomp;
use crate::supervisor::IsolateError;

pub fn run() -> Result<(), IsolateError> {
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let (stdin_tx, stdin_rx) = mpsc::channel();
    thread::Builder::new()
        .name("tysel-worker-stdin".into())
        .spawn(move || {
            let mut stdin = BufReader::new(io::stdin());
            while let Ok(message) = read_message(&mut stdin) {
                if stdin_tx.send(message).is_err() {
                    break;
                }
            }
        })
        .map_err(|err| IsolateError::Worker(err.to_string()))?;

    write_locked(&stdout, &Message::WorkerReady)?;

    let mut config = IsolateConfig::default();
    let mut handler: Option<IsolatePool> = None;
    let mut task_source: Option<String> = None;
    loop {
        let message = stdin_rx.recv().map_err(|err| IsolateError::Worker(err.to_string()))?;
        match message {
            Message::Start {
                memory_limit_bytes,
                cpu_ms_per_turn,
                request_timeout_ms,
                rlimit_as_bytes,
                app,
                json_logs,
            } => {
                config = IsolateConfig { memory_limit_bytes, cpu_ms_per_turn, request_timeout_ms };
                tysel_observability::configure_http_log(app, json_logs);
                rlimit::apply_resource_limits(rlimit_as_bytes)?;
                landlock::apply()?;
                seccomp::apply()?;
                write_locked(&stdout, &Message::Started)?;
            }
            Message::Load { source, secret_names } => {
                let reply = match load_handler(&source, secret_names, config) {
                    Ok(pool) => {
                        handler = Some(pool);
                        Message::Loaded
                    }
                    Err(err) => Message::LoadErr { error: err.to_string() },
                };
                write_locked(&stdout, &reply)?;
            }
            Message::Http { id, method, url, headers, body, request_id } => {
                let reply = match handler.as_ref() {
                    Some(pool) => match pool.dispatch_sync(HttpRequest {
                        method,
                        url,
                        headers,
                        body: body.into_bytes(),
                        request_id,
                    }) {
                        Ok((head, bytes)) => Message::HttpOk {
                            id,
                            status: head.status,
                            headers: head.headers,
                            body: String::from_utf8_lossy(&bytes).into_owned(),
                            websocket: head.websocket,
                        },
                        Err(err) => Message::HttpErr { id, error: err.to_string() },
                    },
                    None => Message::HttpErr { id, error: "handler not loaded".into() },
                };
                write_locked(&stdout, &reply)?;
            }
            Message::TaskLoad { id, source, secret_names } => {
                configure_isolated_capabilities(secret_names);
                let reply = match inspect_task_module(&source, config) {
                    Ok(definitions) => match serde_json::to_string(&definitions) {
                        Ok(definitions_json) => {
                            task_source = Some(source);
                            Message::TaskLoaded { id, definitions_json }
                        }
                        Err(error) => {
                            task_error_reply(id, error.to_string(), TaskErrorKind::Failed)
                        }
                    },
                    Err(error) => task_engine_error_reply(id, error),
                };
                write_locked(&stdout, &reply)?;
            }
            Message::TaskInvoke { id, task_name, input_json, request_id, deadline_ms } => {
                let reply = match task_source.as_deref() {
                    Some(source) => match invoke_task_module(
                        source,
                        &task_name,
                        &input_json,
                        &request_id,
                        deadline_ms,
                        config,
                    ) {
                        Ok(value) => Message::TaskOk { id, value: WireValue::from(value) },
                        Err(error) => task_engine_error_reply(id, error),
                    },
                    None => {
                        task_error_reply(id, "task module not loaded".into(), TaskErrorKind::Failed)
                    }
                };
                write_locked(&stdout, &reply)?;
            }
            Message::Eval { id, source } => {
                let reply = match eval_source(&source, config, &stdout, &stdin_rx) {
                    Ok(value) => Message::EvalOk { id, value: WireValue::from(value) },
                    Err(err) => Message::EvalErr { id, error: err.to_string() },
                };
                write_locked(&stdout, &reply)?;
            }
            Message::Overalloc => {
                let _blob: Vec<u8> = vec![1; 512 * 1024 * 1024];
                write_locked(
                    &stdout,
                    &Message::EvalErr { id: 0, error: "overalloc survived".into() },
                )?;
            }
            Message::Shutdown => return Ok(()),
            // A timed-out eval can still have a CapOk in flight. Drop it.
            Message::CapOk { .. } | Message::CapErr { .. } => {}
            other => {
                return Err(IsolateError::Worker(format!(
                    "unexpected supervisor message {other:?}"
                )));
            }
        }
    }
}

fn load_handler(
    source: &str,
    secret_names: Vec<String>,
    config: IsolateConfig,
) -> Result<IsolatePool, IsolateError> {
    configure_isolated_capabilities(secret_names.clone());
    IsolatePool::spawn(1, source, config).map_err(|err| IsolateError::Worker(err.to_string()))
}

fn configure_isolated_capabilities(secret_names: Vec<String>) {
    tysel_engine_qjs::configure_execution_profile("isolated");
    tysel_engine_qjs::configure_fetch_hosts(Vec::new());
    tysel_engine_qjs::configure_postgres(None, false);
    tysel_engine_qjs::configure_redis(None, false);
    tysel_engine_qjs::configure_fs(Vec::new(), Vec::new(), None);
    let mut secrets = HashMap::new();
    for name in secret_names {
        secrets.insert(name, String::new());
    }
    tysel_engine_qjs::configure_secrets(secrets);
}

fn task_engine_error_reply(id: u64, error: tysel_engine::EngineError) -> Message {
    let kind = match error {
        tysel_engine::EngineError::Interrupted(InterruptReason::Timeout) => TaskErrorKind::TimedOut,
        tysel_engine::EngineError::Interrupted(InterruptReason::Cancelled) => {
            TaskErrorKind::Canceled
        }
        tysel_engine::EngineError::Suspended => TaskErrorKind::Suspended,
        _ => TaskErrorKind::Failed,
    };
    task_error_reply(id, error.to_string(), kind)
}

fn task_error_reply(id: u64, error: String, kind: TaskErrorKind) -> Message {
    Message::TaskErr { id, error, kind }
}

fn eval_source(
    source: &str,
    config: IsolateConfig,
    stdout: &Arc<Mutex<io::Stdout>>,
    stdin_rx: &mpsc::Receiver<Message>,
) -> Result<Value, IsolateError> {
    let cancel = IsolateCancel::new();
    let deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
    let (reactor, mut requests, complete_tx) = open_bridge();
    let stdout_caps = stdout.clone();
    let cancel_caps = cancel.clone();
    let complete_caps = complete_tx.clone();
    thread::Builder::new()
        .name("tysel-worker-caps".into())
        .spawn(move || {
            while let Some(work) = requests.blocking_recv() {
                match work.request {
                    IoRequest::Sleep { id, millis } => {
                        let result = wait_interruptible(
                            Duration::from_millis(millis),
                            &cancel_caps,
                            deadline,
                        );
                        let _ = complete_caps.send(IoCompletion { id, result });
                    }
                    other => {
                        if let Some(message) = cap_call(&other) {
                            let _ = write_locked(&stdout_caps, &message);
                        } else {
                            if let Some((capability, operation)) = other.audit_target() {
                                tysel_observability::log_capability(
                                    capability,
                                    operation,
                                    "denied",
                                    Duration::ZERO,
                                    work.request_id,
                                );
                            }
                            let _ = complete_caps.send(IoCompletion {
                                id: other.id(),
                                result: Err(
                                    "capability is not available in the isolated worker".into()
                                ),
                            });
                        }
                    }
                }
            }
        })
        .map_err(|err| IsolateError::Worker(err.to_string()))?;

    let script = source.to_owned();
    let cancel_eval = cancel.clone();
    let (result_tx, result_rx) = mpsc::channel();
    thread::Builder::new()
        .name("tysel-qjs".into())
        .spawn(move || {
            let result =
                eval_with_reactor_deadline(&script, config, cancel_eval, reactor, deadline);
            let _ = result_tx.send(result);
        })
        .map_err(|err| IsolateError::Worker(err.to_string()))?;

    loop {
        if let Ok(result) = result_rx.try_recv() {
            cancel.cancel();
            return result.map_err(|err| IsolateError::Worker(err.to_string()));
        }
        match stdin_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(Message::CapOk { id, value }) => {
                let _ =
                    complete_tx.send(IoCompletion { id: OpId(id), result: Ok(Value::from(value)) });
            }
            Ok(Message::CapErr { id, error }) => {
                let _ = complete_tx.send(IoCompletion { id: OpId(id), result: Err(error) });
            }
            Ok(Message::Shutdown) => {
                cancel.cancel();
                return Err(IsolateError::Worker("shutdown during eval".into()));
            }
            Ok(other) => {
                cancel.cancel();
                return Err(IsolateError::Worker(format!("unexpected during eval: {other:?}")));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                cancel.cancel();
                return Err(IsolateError::Worker("stdin closed".into()));
            }
        }
    }
}

fn wait_interruptible(
    duration: Duration,
    cancel: &IsolateCancel,
    deadline: Instant,
) -> Result<Value, String> {
    let sleep_until = Instant::now() + duration;
    loop {
        if cancel.is_cancelled() {
            return Err(format!("{:?}", InterruptReason::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(format!("{:?}", InterruptReason::Timeout));
        }
        let now = Instant::now();
        if now >= sleep_until {
            return Ok(Value::Null);
        }
        let slice = (sleep_until - now)
            .min(deadline.saturating_duration_since(now))
            .min(Duration::from_millis(5));
        thread::sleep(slice);
    }
}

fn cap_call(request: &IoRequest) -> Option<Message> {
    match request {
        IoRequest::Sleep { .. } => unreachable!("sleep stays in the worker"),
        other => {
            if !Policy::isolated().allows(other.capability()) {
                return None;
            }
            match other {
                IoRequest::Echo { id, value } => Some(Message::CapCall {
                    id: id.0,
                    op: "echo".into(),
                    args: vec![WireValue::String { v: value.clone() }],
                }),
                IoRequest::SecretRef { id, name } => Some(Message::CapCall {
                    id: id.0,
                    op: "secret.ref".into(),
                    args: vec![WireValue::String { v: name.clone() }],
                }),
                _ => None,
            }
        }
    }
}

fn write_locked(stdout: &Arc<Mutex<io::Stdout>>, message: &Message) -> Result<(), IsolateError> {
    let mut guard = stdout.lock().map_err(|err| IsolateError::Worker(err.to_string()))?;
    write_message(&mut *guard, message)?;
    guard.flush()?;
    Ok(())
}
