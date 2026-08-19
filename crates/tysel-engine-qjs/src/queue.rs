use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, mpsc};
use tysel_engine::{InterruptReason, Value};

pub const STREAM_WINDOW: usize = 16;

type BodyRx = mpsc::Receiver<Result<Vec<u8>, String>>;

static IO_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Process-wide Tokio runtime for isolate host I/O. Multi-thread workers poll
/// independently, so `IsolatePool::spawn` can submit work from a blocked
/// `#[tokio::test]` current-thread runtime without deadlocking. IO is enabled
/// so outbound fetch can connect without borrowing a test runtime.
fn io_handle() -> Handle {
    IO_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_io()
                .enable_time()
                .thread_name("tysel-io")
                .build()
                .expect("shared io runtime")
        })
        .handle()
        .clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OpId(pub u64);

#[derive(Debug)]
pub enum IoRequest {
    Sleep { id: OpId, millis: u64 },
    Echo { id: OpId, value: String },
    SecretRef { id: OpId, name: String },
    ReadBody { id: OpId },
    HttpGet { id: OpId, url: String },
    HttpRead { id: OpId },
}

impl IoRequest {
    pub fn id(&self) -> OpId {
        match self {
            Self::Sleep { id, .. }
            | Self::Echo { id, .. }
            | Self::SecretRef { id, .. }
            | Self::ReadBody { id }
            | Self::HttpGet { id, .. }
            | Self::HttpRead { id } => *id,
        }
    }
}

#[derive(Debug)]
pub struct IoCompletion {
    pub id: OpId,
    pub result: Result<Value, String>,
}

#[derive(Clone, Default)]
pub struct StreamSlot {
    inner: Arc<Mutex<Option<BodyRx>>>,
}

impl StreamSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, rx: BodyRx) {
        *self.inner.blocking_lock() = Some(rx);
    }

    pub fn clear(&self) {
        *self.inner.blocking_lock() = None;
    }

    async fn install_async(&self, rx: BodyRx) {
        *self.inner.lock().await = Some(rx);
    }

    async fn read(&self) -> Result<Option<Vec<u8>>, String> {
        let mut guard = self.inner.lock().await;
        let Some(rx) = guard.as_mut() else {
            return Ok(None);
        };
        match rx.recv().await {
            Some(Ok(chunk)) => Ok(Some(chunk)),
            Some(Err(err)) => Err(err),
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

#[derive(Clone)]
pub struct IoHandle {
    tx: UnboundedSender<IoRequest>,
    next_id: Arc<AtomicU64>,
    pub inbound: StreamSlot,
    pub outbound: StreamSlot,
}

impl IoHandle {
    pub fn submit(&self, request: impl FnOnce(OpId) -> IoRequest) -> OpId {
        let id = OpId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let _ = self.tx.send(request(id));
        id
    }
}

pub struct Reactor {
    pub io: IoHandle,
    pub completions: std::sync::mpsc::Receiver<IoCompletion>,
}

pub fn spawn_reactor(cancel: Arc<AtomicBool>, deadline: Instant) -> Reactor {
    let inbound = StreamSlot::new();
    let outbound = StreamSlot::new();
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let inbound_task = inbound.clone();
    let outbound_task = outbound.clone();
    io_handle().spawn(async move {
        run_reactor(req_rx, done_tx, cancel, deadline, inbound_task, outbound_task).await;
    });

    Reactor {
        io: IoHandle { tx: req_tx, next_id: Arc::new(AtomicU64::new(1)), inbound, outbound },
        completions: done_rx,
    }
}

pub fn spawn_reactor_until_cancel(cancel: Arc<AtomicBool>) -> Reactor {
    spawn_reactor(cancel, Instant::now() + Duration::from_secs(60 * 60 * 24 * 365))
}

/// Split I/O so a process-isolated worker can proxy host calls over IPC.
pub fn open_bridge()
-> (Reactor, UnboundedReceiver<IoRequest>, std::sync::mpsc::Sender<IoCompletion>) {
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    (
        Reactor {
            io: IoHandle {
                tx: req_tx,
                next_id: Arc::new(AtomicU64::new(1)),
                inbound: StreamSlot::new(),
                outbound: StreamSlot::new(),
            },
            completions: done_rx,
        },
        req_rx,
        done_tx,
    )
}

async fn run_reactor(
    mut requests: UnboundedReceiver<IoRequest>,
    completions: std::sync::mpsc::Sender<IoCompletion>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    inbound: StreamSlot,
    outbound: StreamSlot,
) {
    while let Some(request) = requests.recv().await {
        let completions = completions.clone();
        let cancel = cancel.clone();
        let inbound = inbound.clone();
        let outbound = outbound.clone();
        tokio::spawn(async move {
            let completion = execute(request, cancel, deadline, inbound, outbound).await;
            let _ = completions.send(completion);
        });
    }
}

async fn execute(
    request: IoRequest,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    inbound: StreamSlot,
    outbound: StreamSlot,
) -> IoCompletion {
    match request {
        IoRequest::Sleep { id, millis } => IoCompletion {
            id,
            result: wait(Duration::from_millis(millis), &cancel, deadline).await.map_err(io_err),
        },
        IoRequest::Echo { id, value } => {
            let wait_result = wait(Duration::from_millis(1), &cancel, deadline).await;
            IoCompletion { id, result: wait_result.map(|_| Value::String(value)).map_err(io_err) }
        }
        IoRequest::SecretRef { id, name } => {
            IoCompletion { id, result: Ok(Value::String(format!("secret:{name}"))) }
        }
        IoRequest::ReadBody { id } => IoCompletion { id, result: read_chunk(&inbound).await },
        IoRequest::HttpRead { id } => IoCompletion { id, result: read_chunk(&outbound).await },
        IoRequest::HttpGet { id, url } => {
            IoCompletion { id, result: outbound_get(&url, cancel, deadline, outbound).await }
        }
    }
}

async fn read_chunk(slot: &StreamSlot) -> Result<Value, String> {
    match slot.read().await {
        Ok(Some(bytes)) => Ok(Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        Ok(None) => Ok(Value::Null),
        Err(err) => Err(err),
    }
}

async fn outbound_get(
    url: &str,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    outbound: StreamSlot,
) -> Result<Value, String> {
    let uri = url.parse::<hyper::Uri>().map_err(|err| err.to_string())?;
    if uri.scheme_str() != Some("http") {
        return Err("outbound fetch only supports http".into());
    }
    let host = uri.host().ok_or("missing host")?.to_owned();
    let port = uri.port_u16().unwrap_or(80);
    let stream = tokio::select! {
        biased;
        _ = cancelled(&cancel, deadline) => return Err(interrupt_err(&cancel, deadline)),
        result = TcpStream::connect((host.as_str(), port)) => result.map_err(|err| err.to_string())?,
    };
    let (mut sender, conn) = tokio::select! {
        biased;
        _ = cancelled(&cancel, deadline) => return Err(interrupt_err(&cancel, deadline)),
        result = hyper::client::conn::http1::handshake(TokioIo::new(stream)) => {
            result.map_err(|err| err.to_string())?
        }
    };
    io_handle().spawn(async move {
        let _ = conn.await;
    });
    let path = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_owned();
    let host_header = if port == 80 { host } else { format!("{host}:{port}") };
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header(hyper::header::HOST, host_header)
        .body(Empty::<Bytes>::new())
        .map_err(|err| err.to_string())?;
    let response = tokio::select! {
        biased;
        _ = cancelled(&cancel, deadline) => return Err(interrupt_err(&cancel, deadline)),
        result = sender.send_request(request) => result.map_err(|err| err.to_string())?,
    };
    let status = response.status().as_u16();
    let (tx, rx) = mpsc::channel(STREAM_WINDOW);
    outbound.install_async(rx).await;
    io_handle().spawn(async move {
        let _keep_alive = sender;
        pump_http_body(response.into_body(), tx, cancel, deadline).await;
    });
    Ok(Value::Number(f64::from(status)))
}

async fn wait(
    duration: Duration,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Value, InterruptReason> {
    let sleep_until = Instant::now() + duration;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(InterruptReason::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(InterruptReason::Timeout);
        }
        let now = Instant::now();
        if now >= sleep_until {
            return Ok(Value::Null);
        }
        let slice = (sleep_until - now)
            .min(deadline.saturating_duration_since(now))
            .min(Duration::from_millis(5));
        tokio::time::sleep(slice).await;
    }
}

async fn cancelled(cancel: &AtomicBool, deadline: Instant) {
    loop {
        if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn interrupt_err(cancel: &AtomicBool, deadline: Instant) -> String {
    if cancel.load(Ordering::SeqCst) {
        io_err(InterruptReason::Cancelled)
    } else if Instant::now() >= deadline {
        io_err(InterruptReason::Timeout)
    } else {
        io_err(InterruptReason::Cancelled)
    }
}

fn io_err(reason: InterruptReason) -> String {
    format!("{reason:?}")
}

async fn pump_http_body(
    mut body: Incoming,
    tx: mpsc::Sender<Result<Vec<u8>, String>>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
) {
    loop {
        if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
            let _ = tx.send(Err(interrupt_err(&cancel, deadline))).await;
            return;
        }
        let frame = tokio::select! {
            biased;
            _ = cancelled(&cancel, deadline) => {
                let _ = tx.send(Err(interrupt_err(&cancel, deadline))).await;
                return;
            }
            frame = body.frame() => frame,
        };
        match frame {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    if data.is_empty() {
                        continue;
                    }
                    if tx.send(Ok(data.to_vec())).await.is_err() {
                        return;
                    }
                }
            }
            Some(Err(err)) => {
                let _ = tx.send(Err(err.to_string())).await;
                return;
            }
            None => return,
        }
    }
}
