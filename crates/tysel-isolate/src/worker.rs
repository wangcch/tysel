use std::io::{self, BufReader, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use tysel_engine::{IsolateConfig, Value};
use tysel_engine_qjs::{
    IoCompletion, IoRequest, IsolateCancel, OpId, eval_with_reactor, open_bridge,
};
use tysel_ipc::{Message, WireValue, read_message, write_message};

use crate::rlimit;
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
    loop {
        let message = stdin_rx.recv().map_err(|err| IsolateError::Worker(err.to_string()))?;
        match message {
            Message::Start {
                memory_limit_bytes,
                cpu_ms_per_turn,
                request_timeout_ms,
                rlimit_as_bytes,
            } => {
                config = IsolateConfig { memory_limit_bytes, cpu_ms_per_turn, request_timeout_ms };
                rlimit::apply_resource_limits(rlimit_as_bytes)?;
                write_locked(&stdout, &Message::Started)?;
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
            other => {
                return Err(IsolateError::Worker(format!(
                    "unexpected supervisor message {other:?}"
                )));
            }
        }
    }
}

fn eval_source(
    source: &str,
    config: IsolateConfig,
    stdout: &Arc<Mutex<io::Stdout>>,
    stdin_rx: &mpsc::Receiver<Message>,
) -> Result<Value, IsolateError> {
    let (reactor, mut requests, complete_tx) = open_bridge();
    let stdout_caps = stdout.clone();
    thread::Builder::new()
        .name("tysel-worker-caps".into())
        .spawn(move || {
            while let Some(request) = requests.blocking_recv() {
                let _ = write_locked(&stdout_caps, &cap_call(&request));
            }
        })
        .map_err(|err| IsolateError::Worker(err.to_string()))?;

    let script = source.to_owned();
    let (result_tx, result_rx) = mpsc::channel();
    thread::Builder::new()
        .name("tysel-qjs".into())
        .spawn(move || {
            let result = eval_with_reactor(&script, config, IsolateCancel::new(), reactor);
            let _ = result_tx.send(result);
        })
        .map_err(|err| IsolateError::Worker(err.to_string()))?;

    loop {
        if let Ok(result) = result_rx.try_recv() {
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
                return Err(IsolateError::Worker("shutdown during eval".into()));
            }
            Ok(other) => {
                return Err(IsolateError::Worker(format!("unexpected during eval: {other:?}")));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(IsolateError::Worker("stdin closed".into()));
            }
        }
    }
}

fn cap_call(request: &IoRequest) -> Message {
    match request {
        IoRequest::Sleep { id, millis } => Message::CapCall {
            id: id.0,
            op: "sleep".into(),
            args: vec![WireValue::Number { v: *millis as f64 }],
        },
        IoRequest::Echo { id, value } => Message::CapCall {
            id: id.0,
            op: "echo".into(),
            args: vec![WireValue::String { v: value.clone() }],
        },
        IoRequest::SecretRef { id, name } => Message::CapCall {
            id: id.0,
            op: "secret.ref".into(),
            args: vec![WireValue::String { v: name.clone() }],
        },
    }
}

fn write_locked(stdout: &Arc<Mutex<io::Stdout>>, message: &Message) -> Result<(), IsolateError> {
    let mut guard = stdout.lock().map_err(|err| IsolateError::Worker(err.to_string()))?;
    write_message(&mut *guard, message)?;
    guard.flush()?;
    Ok(())
}
