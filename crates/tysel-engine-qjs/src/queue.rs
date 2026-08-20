use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, mpsc};
use tysel_engine::{InterruptReason, Value};
use tysel_policy::Cap;

pub const STREAM_WINDOW: usize = 16;

type BodyRx = mpsc::Receiver<Result<Vec<u8>, String>>;

static IO_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Process-wide Tokio runtime for isolate host I/O. Multi-thread workers poll
/// independently, so `IsolatePool::spawn` can submit work from a blocked
/// `#[tokio::test]` current-thread runtime without deadlocking. IO is enabled
/// so outbound fetch can connect without borrowing a test runtime.
pub(crate) fn io_handle() -> Handle {
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
    HttpGet { id: OpId, url: String, method: String, headers_json: String, body: String },
    HttpRead { id: OpId },
    WsRead { id: OpId },
    WsSend { id: OpId, data: String },
    WsClose { id: OpId },
    SqliteExec { id: OpId, sql: String, params_json: String },
    SqliteQuery { id: OpId, sql: String, params_json: String },
    PostgresExec { id: OpId, sql: String, params_json: String },
    PostgresQuery { id: OpId, sql: String, params_json: String },
    FsRead { id: OpId, path: String },
    FsWrite { id: OpId, path: String, data: String },
    LlmGenerate { id: OpId, request_json: String },
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
            | Self::SqliteQuery { id, .. }
            | Self::PostgresExec { id, .. }
            | Self::PostgresQuery { id, .. }
            | Self::FsRead { id, .. }
            | Self::FsWrite { id, .. }
            | Self::LlmGenerate { id, .. } => *id,
        }
    }

    pub fn capability(&self) -> Cap {
        match self {
            Self::Sleep { .. } => Cap::Sleep,
            Self::Echo { .. } => Cap::Echo,
            Self::SecretRef { .. } => Cap::SecretRef,
            Self::ReadBody { .. } => Cap::ReadBody,
            Self::HttpGet { .. } | Self::HttpRead { .. } => Cap::Fetch,
            Self::WsRead { .. } | Self::WsSend { .. } | Self::WsClose { .. } => Cap::WebSocket,
            Self::SqliteExec { .. } | Self::SqliteQuery { .. } => Cap::Sqlite,
            Self::PostgresExec { .. } | Self::PostgresQuery { .. } => Cap::Postgres,
            Self::FsRead { .. } | Self::FsWrite { .. } => Cap::Fs,
            Self::LlmGenerate { .. } => Cap::Llm,
        }
    }

    /// Capability audit identity. Sleep, echo, and stream-chunk reads are omitted.
    pub fn audit_target(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::HttpGet { .. } => Some(("fetch", "request")),
            Self::SqliteExec { .. } => Some(("sqlite", "exec")),
            Self::SqliteQuery { .. } => Some(("sqlite", "query")),
            Self::PostgresExec { .. } => Some(("postgres", "exec")),
            Self::PostgresQuery { .. } => Some(("postgres", "query")),
            Self::FsRead { .. } => Some(("fs", "read")),
            Self::FsWrite { .. } => Some(("fs", "write")),
            Self::LlmGenerate { .. } => Some(("llm", "generate")),
            Self::SecretRef { .. } => Some(("secrets", "ref")),
            Self::WsSend { .. } => Some(("websocket", "send")),
            Self::WsClose { .. } => Some(("websocket", "close")),
            Self::Sleep { .. }
            | Self::Echo { .. }
            | Self::ReadBody { .. }
            | Self::HttpRead { .. }
            | Self::WsRead { .. } => None,
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
    tx: UnboundedSender<IoWork>,
    next_id: Arc<AtomicU64>,
    request_id: Arc<AtomicU64>,
    pub inbound: StreamSlot,
    pub outbound: StreamSlot,
    pub ws_in: StreamSlot,
    pub ws_out: SendSlot,
}

/// One host I/O op plus the HTTP request id that submitted it.
pub struct IoWork {
    pub request: IoRequest,
    pub request_id: u64,
}

impl IoHandle {
    pub fn bind_request(&self, request_id: u64) {
        self.request_id.store(request_id, Ordering::Relaxed);
    }

    pub fn submit(&self, request: impl FnOnce(OpId) -> IoRequest) -> OpId {
        let id = OpId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let request_id = self.request_id.load(Ordering::Relaxed);
        let _ = self.tx.send(IoWork { request: request(id), request_id });
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
            request_id: Arc::new(AtomicU64::new(0)),
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
pub fn open_bridge() -> (Reactor, UnboundedReceiver<IoWork>, std::sync::mpsc::Sender<IoCompletion>)
{
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    (
        Reactor {
            io: IoHandle {
                tx: req_tx,
                next_id: Arc::new(AtomicU64::new(1)),
                request_id: Arc::new(AtomicU64::new(0)),
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
    mut requests: UnboundedReceiver<IoWork>,
    completions: std::sync::mpsc::Sender<IoCompletion>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    slots: IoSlots,
) {
    while let Some(work) = requests.recv().await {
        let completions = completions.clone();
        let cancel = cancel.clone();
        let slots = IoSlots {
            inbound: slots.inbound.clone(),
            outbound: slots.outbound.clone(),
            ws_in: slots.ws_in.clone(),
            ws_out: slots.ws_out.clone(),
        };
        tokio::spawn(async move {
            let completion = execute(work.request, work.request_id, cancel, deadline, slots).await;
            let _ = completions.send(completion);
        });
    }
}

async fn execute(
    request: IoRequest,
    request_id: u64,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    slots: IoSlots,
) -> IoCompletion {
    let audit = request.audit_target();
    let started = Instant::now();
    if let Err(error) = crate::trust::require(request.capability()) {
        audit_log(audit, "denied", started, request_id);
        return IoCompletion { id: request.id(), result: Err(error) };
    }
    let completion = match request {
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
        IoRequest::HttpGet { id, url, method, headers_json, body } => IoCompletion {
            id,
            result: outbound_fetch(
                &method,
                &url,
                &headers_json,
                &body,
                cancel,
                deadline,
                slots.outbound,
            )
            .await,
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
        IoRequest::PostgresExec { id, sql, params_json } => IoCompletion {
            id,
            result: postgres_op(sql, params_json, false, cancel, deadline).await,
        },
        IoRequest::PostgresQuery { id, sql, params_json } => {
            IoCompletion { id, result: postgres_op(sql, params_json, true, cancel, deadline).await }
        }
        IoRequest::FsRead { id, path } => IoCompletion {
            id,
            result: run_blocking(cancel, deadline, move || {
                tysel_cap_fs::read(&path).map(Value::String)
            })
            .await,
        },
        IoRequest::FsWrite { id, path, data } => IoCompletion {
            id,
            result: run_blocking(cancel, deadline, move || {
                tysel_cap_fs::write(&path, &data).map(|()| Value::Null)
            })
            .await,
        },
        IoRequest::LlmGenerate { id, request_json } => IoCompletion {
            id,
            result: crate::llm::generate(request_json, request_id, id, cancel, deadline).await,
        },
    };
    let result = if completion.result.is_ok() { "ok" } else { "error" };
    audit_log(audit, result, started, request_id);
    completion
}

fn audit_log(
    audit: Option<(&'static str, &'static str)>,
    result: &str,
    started: Instant,
    request_id: u64,
) {
    if let Some((capability, operation)) = audit {
        tysel_observability::log_capability(
            capability,
            operation,
            result,
            started.elapsed(),
            request_id,
        );
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

async fn postgres_op(
    sql: String,
    params_json: String,
    query: bool,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Value, String> {
    tokio::select! {
        biased;
        result = async {
            if query {
                tysel_cap_postgres::query(&sql, &params_json).await
            } else {
                tysel_cap_postgres::exec(&sql, &params_json).await.map(Value::Number)
            }
        } => result,
        _ = cancelled(&cancel, deadline) => Err(interrupt_err(&cancel, deadline)),
    }
}

async fn run_blocking<T, F>(
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
        _ = cancelled(&cancel, deadline) => Err(interrupt_err(&cancel, deadline)),
    }
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
const MAX_OUTBOUND_BODY: usize = 16 * 1024 * 1024;

struct Hop {
    response: hyper::Response<Incoming>,
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
}

async fn outbound_fetch(
    method: &str,
    url: &str,
    headers_json: &str,
    body: &str,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    outbound: StreamSlot,
) -> Result<Value, String> {
    let mut method = normalize_method(method)?;
    let mut headers = crate::fetch_policy::expand_headers_json(headers_json)?;
    let mut body = request_body(&method, body)?;
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let hop =
            fetch_hop(&method, &url, &headers.headers, body.clone(), &cancel, deadline).await?;
        let status = hop.response.status();
        if status.is_redirection()
            && let Some(location) = hop
                .response
                .headers()
                .get(hyper::header::LOCATION)
                .and_then(|value| value.to_str().ok())
        {
            let next = resolve_redirect(&url, location)?;
            if !crate::fetch_policy::same_origin(&url, &next)? {
                crate::fetch_policy::strip_credentials_for_cross_origin(&mut headers);
            }
            url = next;
            if matches!(status.as_u16(), 301..=303) && method != "HEAD" {
                method = "GET".into();
                body = Bytes::new();
            }
            continue;
        }
        let code = status.as_u16();
        let headers_json = response_headers_json(hop.response.headers());
        let (tx, rx) = mpsc::channel(STREAM_WINDOW);
        outbound.install_async(rx).await;
        io_handle().spawn(async move {
            let _keep_alive = hop.sender;
            pump_http_body(hop.response.into_body(), tx, cancel, deadline).await;
        });
        return Ok(Value::Record(vec![
            ("status".into(), Value::Number(f64::from(code))),
            ("headers".into(), Value::String(headers_json)),
        ]));
    }
    Err("too many redirects".into())
}

fn response_headers_json(headers: &hyper::HeaderMap) -> String {
    let mut pairs = Vec::new();
    for (name, value) in headers.iter() {
        if crate::fetch_policy::skip_response_header(name.as_str()) {
            continue;
        }
        let Ok(value) = value.to_str() else {
            continue;
        };
        pairs.push((name.as_str(), value));
    }
    serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".into())
}

fn normalize_method(method: &str) -> Result<String, String> {
    let method = method.to_ascii_uppercase();
    match method.as_str() {
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" => Ok(method),
        _ => Err("outbound fetch only supports GET, HEAD, POST, PUT, PATCH, and DELETE".into()),
    }
}

fn request_body(method: &str, body: &str) -> Result<Bytes, String> {
    if method == "GET" || method == "HEAD" {
        return Ok(Bytes::new());
    }
    if body.len() > MAX_OUTBOUND_BODY {
        return Err(format!("request body exceeds {MAX_OUTBOUND_BODY} bytes"));
    }
    Ok(Bytes::from(body.to_owned()))
}

async fn fetch_hop(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Bytes,
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
        handshake_and_send(
            TokioIo::new(tls),
            OutboundHop { method, path: &path, host_header: &host_header, headers, body },
            cancel,
            deadline,
        )
        .await
    } else {
        handshake_and_send(
            TokioIo::new(stream),
            OutboundHop { method, path: &path, host_header: &host_header, headers, body },
            cancel,
            deadline,
        )
        .await
    }
}

struct OutboundHop<'a> {
    method: &'a str,
    path: &'a str,
    host_header: &'a str,
    headers: &'a [(String, String)],
    body: Bytes,
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
    hop: OutboundHop<'_>,
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
    let mut builder = Request::builder()
        .method(hop.method)
        .uri(hop.path)
        .header(hyper::header::HOST, hop.host_header);
    for (name, value) in hop.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let request = builder.body(Full::new(hop.body)).map_err(|err| err.to_string())?;
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

pub(crate) async fn cancelled(cancel: &AtomicBool, deadline: Instant) {
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
