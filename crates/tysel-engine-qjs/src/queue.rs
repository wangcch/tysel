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
    HttpGet { id: OpId, url: String, method: String },
    HttpRead { id: OpId },
    WsRead { id: OpId },
    WsSend { id: OpId, data: String },
    WsClose { id: OpId },
    SqliteExec { id: OpId, sql: String, params_json: String },
    SqliteQuery { id: OpId, sql: String, params_json: String },
}

impl IoRequest {
    pub fn id(&self) -> OpId {
        match self {
            Self::Sleep { id, .. }
            | Self::Echo { id, .. }
            | Self::SecretRef { id, .. }
            | Self::ReadBody { id }
            | Self::HttpGet { id, .. }
            | Self::HttpRead { id }
            | Self::WsRead { id }
            | Self::WsSend { id, .. }
            | Self::WsClose { id }
            | Self::SqliteExec { id, .. }
            | Self::SqliteQuery { id, .. } => *id,
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

#[derive(Clone, Default)]
pub struct SendSlot {
    inner: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
}

impl SendSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, tx: mpsc::Sender<Vec<u8>>) {
        *self.inner.blocking_lock() = Some(tx);
    }

    pub fn clear(&self) {
        *self.inner.blocking_lock() = None;
    }

    async fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        let tx = {
            let guard = self.inner.lock().await;
            guard.clone().ok_or_else(|| "websocket is not connected".to_string())?
        };
        tx.send(bytes).await.map_err(|_| "websocket closed".into())
    }

    async fn close(&self) {
        *self.inner.lock().await = None;
    }
}

#[derive(Clone)]
pub struct IoHandle {
    tx: UnboundedSender<IoRequest>,
    next_id: Arc<AtomicU64>,
    pub inbound: StreamSlot,
    pub outbound: StreamSlot,
    pub ws_in: StreamSlot,
    pub ws_out: SendSlot,
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
    let ws_in = StreamSlot::new();
    let ws_out = SendSlot::new();
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let inbound_task = inbound.clone();
    let outbound_task = outbound.clone();
    let ws_in_task = ws_in.clone();
    let ws_out_task = ws_out.clone();
    io_handle().spawn(async move {
        run_reactor(
            req_rx,
            done_tx,
            cancel,
            deadline,
            IoSlots {
                inbound: inbound_task,
                outbound: outbound_task,
                ws_in: ws_in_task,
                ws_out: ws_out_task,
            },
        )
        .await;
    });

    Reactor {
        io: IoHandle {
            tx: req_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            inbound,
            outbound,
            ws_in,
            ws_out,
        },
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
                ws_in: StreamSlot::new(),
                ws_out: SendSlot::new(),
            },
            completions: done_rx,
        },
        req_rx,
        done_tx,
    )
}

struct IoSlots {
    inbound: StreamSlot,
    outbound: StreamSlot,
    ws_in: StreamSlot,
    ws_out: SendSlot,
}

async fn run_reactor(
    mut requests: UnboundedReceiver<IoRequest>,
    completions: std::sync::mpsc::Sender<IoCompletion>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    slots: IoSlots,
) {
    while let Some(request) = requests.recv().await {
        let completions = completions.clone();
        let cancel = cancel.clone();
        let slots = IoSlots {
            inbound: slots.inbound.clone(),
            outbound: slots.outbound.clone(),
            ws_in: slots.ws_in.clone(),
            ws_out: slots.ws_out.clone(),
        };
        tokio::spawn(async move {
            let completion = execute(request, cancel, deadline, slots).await;
            let _ = completions.send(completion);
        });
    }
}

async fn execute(
    request: IoRequest,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    slots: IoSlots,
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
            IoCompletion { id, result: crate::secrets::refer(&name) }
        }
        IoRequest::ReadBody { id } => IoCompletion { id, result: read_chunk(&slots.inbound).await },
        IoRequest::HttpRead { id } => {
            IoCompletion { id, result: read_chunk(&slots.outbound).await }
        }
        IoRequest::HttpGet { id, url, method } => IoCompletion {
            id,
            result: outbound_fetch(&method, &url, cancel, deadline, slots.outbound).await,
        },
        IoRequest::WsRead { id } => IoCompletion { id, result: read_chunk(&slots.ws_in).await },
        IoRequest::WsSend { id, data } => IoCompletion {
            id,
            result: slots.ws_out.send(data.into_bytes()).await.map(|()| Value::Null),
        },
        IoRequest::WsClose { id } => {
            slots.ws_out.close().await;
            IoCompletion { id, result: Ok(Value::Null) }
        }
        IoRequest::SqliteExec { id, sql, params_json } => {
            IoCompletion { id, result: sqlite_op(sql, params_json, false, cancel, deadline).await }
        }
        IoRequest::SqliteQuery { id, sql, params_json } => {
            IoCompletion { id, result: sqlite_op(sql, params_json, true, cancel, deadline).await }
        }
    }
}

async fn sqlite_op(
    sql: String,
    params_json: String,
    query: bool,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Value, String> {
    wait_or_interrupt(cancel.clone(), deadline, tysel_cap_sqlite::ensure_ready).await?;
    wait_or_interrupt(cancel, deadline, move || {
        if query {
            tysel_cap_sqlite::query(&sql, &params_json)
        } else {
            tysel_cap_sqlite::exec(&sql, &params_json).map(Value::Number)
        }
    })
    .await
}

async fn wait_or_interrupt<T, F>(
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    work: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if cancel.load(Ordering::SeqCst) {
        return Err(io_err(InterruptReason::Cancelled));
    }
    if Instant::now() >= deadline {
        return Err(io_err(InterruptReason::Timeout));
    }
    let cancel_flag = cancel.clone();
    let task = tokio::task::spawn_blocking(move || {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(io_err(InterruptReason::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(io_err(InterruptReason::Timeout));
        }
        work()
    });
    tokio::pin!(task);
    tokio::select! {
        biased;
        result = &mut task => result.map_err(|err| err.to_string())?,
        _ = cancelled(&cancel, deadline) => {
            tysel_cap_sqlite::interrupt();
            match task.await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) | Err(_) => Err(interrupt_err(&cancel, deadline)),
            }
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

const MAX_REDIRECTS: u8 = 20;

struct Hop {
    response: hyper::Response<Incoming>,
    sender: hyper::client::conn::http1::SendRequest<Empty<Bytes>>,
}

async fn outbound_fetch(
    method: &str,
    url: &str,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    outbound: StreamSlot,
) -> Result<Value, String> {
    let mut method = method.to_ascii_uppercase();
    if method != "GET" && method != "HEAD" {
        return Err("outbound fetch only supports GET and HEAD".into());
    }
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let hop = fetch_hop(&method, &url, &cancel, deadline).await?;
        let status = hop.response.status();
        if status.is_redirection() {
            if let Some(location) = hop
                .response
                .headers()
                .get(hyper::header::LOCATION)
                .and_then(|value| value.to_str().ok())
            {
                url = resolve_redirect(&url, location)?;
                if matches!(status.as_u16(), 301..=303) && method != "HEAD" {
                    method = "GET".into();
                }
                continue;
            }
        }
        let code = status.as_u16();
        let (tx, rx) = mpsc::channel(STREAM_WINDOW);
        outbound.install_async(rx).await;
        io_handle().spawn(async move {
            let _keep_alive = hop.sender;
            pump_http_body(hop.response.into_body(), tx, cancel, deadline).await;
        });
        return Ok(Value::Number(f64::from(code)));
    }
    Err("too many redirects".into())
}

async fn fetch_hop(
    method: &str,
    url: &str,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Hop, String> {
    let uri: hyper::Uri =
        url.parse().map_err(|err: hyper::http::uri::InvalidUri| err.to_string())?;
    let https = match uri.scheme_str() {
        Some("http") => false,
        Some("https") => true,
        _ => return Err("outbound fetch only supports http and https".into()),
    };
    let host = uri.host().ok_or("missing host")?.to_owned();
    crate::fetch_policy::host_permitted(&host)?;
    let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
    let stream = tokio::select! {
        biased;
        _ = cancelled(cancel, deadline) => return Err(interrupt_err(cancel, deadline)),
        result = TcpStream::connect((host.as_str(), port)) => result.map_err(|err| err.to_string())?,
    };
    let path = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_owned();
    let host_header = if (!https && port == 80) || (https && port == 443) {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    if https {
        let tls = tls_connect(&host, stream, cancel, deadline).await?;
        handshake_and_send(TokioIo::new(tls), method, &path, &host_header, cancel, deadline).await
    } else {
        handshake_and_send(TokioIo::new(stream), method, &path, &host_header, cancel, deadline)
            .await
    }
}

async fn tls_connect(
    server_name: &str,
    stream: TcpStream,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<tokio_native_tls::TlsStream<TcpStream>, String> {
    let connector = tokio_native_tls::TlsConnector::from(
        native_tls::TlsConnector::new().map_err(|err| err.to_string())?,
    );
    tokio::select! {
        biased;
        _ = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
        result = connector.connect(server_name, stream) => result.map_err(|err| err.to_string()),
    }
}

async fn handshake_and_send<I>(
    io: I,
    method: &str,
    path: &str,
    host_header: &str,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Hop, String>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = tokio::select! {
        biased;
        _ = cancelled(cancel, deadline) => return Err(interrupt_err(cancel, deadline)),
        result = hyper::client::conn::http1::handshake(io) => result.map_err(|err| err.to_string())?,
    };
    io_handle().spawn(async move {
        let _ = conn.await;
    });
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, host_header)
        .body(Empty::<Bytes>::new())
        .map_err(|err| err.to_string())?;
    let response = tokio::select! {
        biased;
        _ = cancelled(cancel, deadline) => return Err(interrupt_err(cancel, deadline)),
        result = sender.send_request(request) => result.map_err(|err| err.to_string())?,
    };
    Ok(Hop { response, sender })
}

fn resolve_redirect(current: &str, location: &str) -> Result<String, String> {
    let location = location.trim();
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_owned());
    }
    let base: hyper::Uri =
        current.parse().map_err(|err: hyper::http::uri::InvalidUri| err.to_string())?;
    let scheme = base.scheme_str().ok_or("missing scheme")?;
    let authority = base.authority().ok_or("missing host")?;
    if let Some(rest) = location.strip_prefix('/') {
        return Ok(format!("{scheme}://{authority}/{rest}"));
    }
    let prefix = base.path().rsplit_once('/').map(|(head, _)| head).unwrap_or("");
    Ok(format!("{scheme}://{authority}{prefix}/{location}"))
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
