use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tysel_engine::{InterruptReason, Value};
use tysel_policy::Cap;

pub const STREAM_WINDOW: usize = 16;
pub const MAX_PENDING_IO_OPS: usize = 256;
pub const MAX_PENDING_IO_BYTES: usize = 32 * 1024 * 1024;
const IO_BUDGET_ERROR: &str = "host I/O budget exceeded";

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectMode {
    Follow,
    Error,
    Manual,
}

#[derive(Debug)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers_json: String,
    pub body: Bytes,
    pub redirect: RedirectMode,
}

#[derive(Debug)]
pub enum IoRequest {
    Sleep { id: OpId, millis: u64 },
    Echo { id: OpId, value: String },
    SecretRef { id: OpId, name: String },
    ReadBody { id: OpId },
    HttpGet { id: OpId, request: FetchRequest },
    HttpRead { id: OpId, body_id: u64 },
    ResponseClosed { id: OpId, tx: mpsc::Sender<Vec<u8>>, stop: tokio::sync::oneshot::Receiver<()> },
    ResponseWrite { id: OpId, tx: mpsc::Sender<Vec<u8>>, bytes: Vec<u8> },
    WsRead { id: OpId },
    WsSend { id: OpId, data: String },
    WsClose { id: OpId },
    WsConnect { id: OpId, url: String },
    WsClientRead { id: OpId },
    WsClientSend { id: OpId, data: String },
    WsClientClose { id: OpId },
    SqliteExec { id: OpId, sql: String, params_json: String },
    SqliteQuery { id: OpId, sql: String, params_json: String },
    PostgresExec { id: OpId, sql: String, params_json: String },
    PostgresQuery { id: OpId, sql: String, params_json: String },
    RedisGet { id: OpId, key: String },
    RedisSet { id: OpId, key: String, value: String, ttl_seconds: Option<u64> },
    RedisDel { id: OpId, keys_json: String },
    RedisExists { id: OpId, key: String },
    RedisExpire { id: OpId, key: String, ttl_seconds: u64 },
    FsRead { id: OpId, path: String },
    FsWrite { id: OpId, path: String, data: String },
    LlmGenerate { id: OpId, request_json: String },
}

impl IoRequest {
    fn retained_bytes(&self) -> usize {
        let buffers = match self {
            Self::Echo { value, .. } => value.capacity(),
            Self::SecretRef { name, .. } => name.capacity(),
            Self::HttpGet { request, .. } => {
                request.url.capacity()
                    + request.method.capacity()
                    + request.headers_json.capacity()
                    + request.body.len()
            }
            Self::ResponseWrite { bytes, .. } => bytes.capacity(),
            Self::WsSend { data, .. } | Self::WsClientSend { data, .. } => data.capacity(),
            Self::WsConnect { url, .. } => url.capacity(),
            Self::SqliteExec { sql, params_json, .. }
            | Self::SqliteQuery { sql, params_json, .. }
            | Self::PostgresExec { sql, params_json, .. }
            | Self::PostgresQuery { sql, params_json, .. } => {
                sql.capacity() + params_json.capacity()
            }
            Self::RedisGet { key, .. }
            | Self::RedisExists { key, .. }
            | Self::RedisExpire { key, .. } => key.capacity(),
            Self::RedisSet { key, value, .. } => key.capacity() + value.capacity(),
            Self::RedisDel { keys_json, .. } => keys_json.capacity(),
            Self::FsRead { path, .. } => path.capacity(),
            Self::FsWrite { path, data, .. } => path.capacity() + data.capacity(),
            Self::LlmGenerate { request_json, .. } => request_json.capacity(),
            _ => 0,
        };
        size_of::<Self>() + buffers
    }

    pub fn id(&self) -> OpId {
        match self {
            Self::Sleep { id, .. }
            | Self::Echo { id, .. }
            | Self::SecretRef { id, .. }
            | Self::ReadBody { id }
            | Self::HttpGet { id, .. }
            | Self::HttpRead { id, .. }
            | Self::ResponseClosed { id, .. }
            | Self::ResponseWrite { id, .. }
            | Self::WsRead { id }
            | Self::WsSend { id, .. }
            | Self::WsClose { id }
            | Self::WsConnect { id, .. }
            | Self::WsClientRead { id }
            | Self::WsClientSend { id, .. }
            | Self::WsClientClose { id }
            | Self::SqliteExec { id, .. }
            | Self::SqliteQuery { id, .. }
            | Self::PostgresExec { id, .. }
            | Self::PostgresQuery { id, .. }
            | Self::RedisGet { id, .. }
            | Self::RedisSet { id, .. }
            | Self::RedisDel { id, .. }
            | Self::RedisExists { id, .. }
            | Self::RedisExpire { id, .. }
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
            Self::ReadBody { .. } | Self::ResponseWrite { .. } | Self::ResponseClosed { .. } => {
                Cap::ReadBody
            }
            Self::HttpGet { .. } | Self::HttpRead { .. } => Cap::Fetch,
            Self::WsRead { .. }
            | Self::WsSend { .. }
            | Self::WsClose { .. }
            | Self::WsConnect { .. }
            | Self::WsClientRead { .. }
            | Self::WsClientSend { .. }
            | Self::WsClientClose { .. } => Cap::WebSocket,
            Self::SqliteExec { .. } | Self::SqliteQuery { .. } => Cap::Sqlite,
            Self::PostgresExec { .. } | Self::PostgresQuery { .. } => Cap::Postgres,
            Self::RedisGet { .. }
            | Self::RedisSet { .. }
            | Self::RedisDel { .. }
            | Self::RedisExists { .. }
            | Self::RedisExpire { .. } => Cap::Redis,
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
            Self::RedisGet { .. } => Some(("redis", "get")),
            Self::RedisSet { .. } => Some(("redis", "set")),
            Self::RedisDel { .. } => Some(("redis", "del")),
            Self::RedisExists { .. } => Some(("redis", "exists")),
            Self::RedisExpire { .. } => Some(("redis", "expire")),
            Self::FsRead { .. } => Some(("fs", "read")),
            Self::FsWrite { .. } => Some(("fs", "write")),
            Self::LlmGenerate { .. } => Some(("llm", "generate")),
            Self::SecretRef { .. } => Some(("secrets", "ref")),
            Self::WsSend { .. } => Some(("websocket", "send")),
            Self::WsClose { .. } => Some(("websocket", "close")),
            Self::WsConnect { .. } => Some(("websocket", "connect")),
            Self::WsClientSend { .. } => Some(("websocket", "client_send")),
            Self::WsClientClose { .. } => Some(("websocket", "client_close")),
            Self::Sleep { .. }
            | Self::Echo { .. }
            | Self::ReadBody { .. }
            | Self::HttpRead { .. }
            | Self::ResponseClosed { .. }
            | Self::ResponseWrite { .. }
            | Self::WsRead { .. } => None,
            Self::WsClientRead { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct IoCompletion {
    pub id: OpId,
    pub result: Result<Value, String>,
}

#[derive(Clone)]
pub struct StreamSlot {
    inner: Arc<Mutex<Option<BodyRx>>>,
    generation: Arc<AtomicU64>,
}

impl Default for StreamSlot {
    fn default() -> Self {
        Self { inner: Arc::new(Mutex::new(None)), generation: Arc::new(AtomicU64::new(0)) }
    }
}

#[derive(Clone)]
pub struct StreamRegistry {
    inner: Arc<Mutex<StreamRegistryState>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Default)]
struct StreamRegistryState {
    streams: HashMap<u64, BodyRx>,
    in_flight: HashSet<u64>,
    cancelled: HashSet<u64>,
    generation: u64,
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamRegistryState::default())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&self, id: u64) {
        let mut state = self.inner.blocking_lock();
        if state.streams.remove(&id).is_none() && state.in_flight.contains(&id) {
            state.cancelled.insert(id);
        }
    }

    pub fn clear_all(&self) {
        let mut state = self.inner.blocking_lock();
        state.generation = state.generation.saturating_add(1);
        state.streams.clear();
        state.cancelled.clear();
    }

    async fn install(&self, rx: BodyRx) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner.lock().await.streams.insert(id, rx);
        id
    }

    async fn read(
        &self,
        id: u64,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>, String> {
        let (generation, mut rx) = {
            let mut state = self.inner.lock().await;
            let rx = state
                .streams
                .remove(&id)
                .ok_or_else(|| "unknown or consumed response body".to_string())?;
            state.in_flight.insert(id);
            (state.generation, rx)
        };
        let received = tokio::select! {
            biased;
            () = cancelled(cancel, deadline) => None,
            received = rx.recv() => Some(received),
        };
        let mut state = self.inner.lock().await;
        state.in_flight.remove(&id);
        let interrupted = received.is_none();
        let explicitly_cancelled = state.cancelled.remove(&id);
        let invalidated = interrupted || state.generation != generation || explicitly_cancelled;
        if invalidated {
            return Err(if interrupted {
                interrupt_err(cancel, deadline)
            } else {
                io_err(InterruptReason::Cancelled)
            });
        }
        match received.expect("non-interrupted response body read") {
            Some(Ok(chunk)) => {
                state.streams.insert(id, rx);
                Ok(Some(chunk))
            }
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    async fn clear_async(&self, id: u64) {
        let mut state = self.inner.lock().await;
        if state.streams.remove(&id).is_none() && state.in_flight.contains(&id) {
            state.cancelled.insert(id);
        }
    }
}

impl StreamSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, rx: BodyRx) {
        let mut slot = self.inner.blocking_lock();
        self.generation.fetch_add(1, Ordering::SeqCst);
        *slot = Some(rx);
    }

    pub fn clear(&self) {
        let mut slot = self.inner.blocking_lock();
        self.generation.fetch_add(1, Ordering::SeqCst);
        *slot = None;
    }

    async fn read(
        &self,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>, String> {
        let (generation, mut rx) = {
            let mut slot = self.inner.lock().await;
            let generation = self.generation.load(Ordering::SeqCst);
            let Some(rx) = slot.take() else {
                return Ok(None);
            };
            (generation, rx)
        };
        let received = tokio::select! {
            biased;
            () = cancelled(cancel, deadline) => return Err(interrupt_err(cancel, deadline)),
            received = rx.recv() => received,
        };
        match received {
            Some(Ok(chunk)) => {
                let mut slot = self.inner.lock().await;
                if self.generation.load(Ordering::SeqCst) != generation {
                    return Err(io_err(InterruptReason::Cancelled));
                }
                *slot = Some(rx);
                Ok(Some(chunk))
            }
            Some(Err(err)) => Err(err),
            None => Ok(None),
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

    async fn send(
        &self,
        bytes: Vec<u8>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        let tx = {
            let guard = self.inner.lock().await;
            guard.clone().ok_or_else(|| "websocket is not connected".to_string())?
        };
        tokio::select! {
            biased;
            () = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
            result = tx.send(bytes) => result.map_err(|_| "websocket closed".into()),
        }
    }

    async fn close(&self) {
        *self.inner.lock().await = None;
    }
}

enum ClientWebSocketCommand {
    Read { reply: oneshot::Sender<Result<Value, String>> },
    Send { data: String, reply: oneshot::Sender<Result<(), String>> },
    Close { reply: oneshot::Sender<Result<(), String>> },
}

#[derive(Default)]
struct ClientWebSocketState {
    connecting: bool,
    commands: Option<mpsc::UnboundedSender<ClientWebSocketCommand>>,
}

#[derive(Clone)]
pub struct ClientWebSocketSlot {
    generation: Arc<AtomicU64>,
    state: Arc<StdMutex<ClientWebSocketState>>,
    cleared: watch::Sender<u64>,
}

impl Default for ClientWebSocketSlot {
    fn default() -> Self {
        let (cleared, _) = watch::channel(0);
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            state: Arc::new(StdMutex::new(ClientWebSocketState::default())),
            cleared,
        }
    }
}

impl ClientWebSocketSlot {
    pub fn clear(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        let _ = self.cleared.send(generation);
        let mut state = self.state.lock().expect("client websocket state");
        state.connecting = false;
        state.commands = None;
    }

    async fn connect(
        &self,
        url: String,
        cancel: &Arc<AtomicBool>,
        deadline: Instant,
    ) -> Result<(), String> {
        let generation = self.generation.load(Ordering::SeqCst);
        let mut cleared = self.cleared.subscribe();
        let uri: hyper::Uri =
            url.parse().map_err(|err: hyper::http::uri::InvalidUri| err.to_string())?;
        match uri.scheme_str() {
            Some("ws" | "wss") => {}
            _ => return Err("outbound WebSocket only supports ws and wss".into()),
        }
        let host = uri.host().ok_or("missing host")?;
        crate::fetch_policy::host_permitted(host)?;
        {
            let mut state = self.state.lock().expect("client websocket state");
            if state.connecting || state.commands.is_some() {
                return Err("an outbound WebSocket is already connected".into());
            }
            state.connecting = true;
        }
        let connected = tokio::select! {
            biased;
            _ = cleared.changed() => Err("outbound WebSocket request ended".to_string()),
            _ = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
            result = connect_async(url) => result.map_err(|err| err.to_string()),
        };
        let (socket, _) = match connected {
            Ok(socket) => socket,
            Err(error) => {
                let mut state = self.state.lock().expect("client websocket state");
                if self.generation.load(Ordering::SeqCst) == generation {
                    state.connecting = false;
                }
                return Err(error);
            }
        };
        let (commands, receiver) = mpsc::unbounded_channel();
        {
            let mut state = self.state.lock().expect("client websocket state");
            if self.generation.load(Ordering::SeqCst) != generation {
                state.connecting = false;
                return Err("outbound WebSocket request ended".into());
            }
            state.connecting = false;
            state.commands = Some(commands);
        }
        io_handle().spawn(run_client_websocket(
            socket,
            receiver,
            cleared,
            self.state.clone(),
            self.generation.clone(),
            generation,
        ));
        Ok(())
    }

    async fn send(
        &self,
        data: String,
        cancel: &Arc<AtomicBool>,
        deadline: Instant,
    ) -> Result<(), String> {
        let commands = self.commands()?;
        let (reply, result) = oneshot::channel();
        commands
            .send(ClientWebSocketCommand::Send { data, reply })
            .map_err(|_| "outbound WebSocket is closed".to_string())?;
        tokio::select! {
            biased;
            _ = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
            result = result => result.map_err(|_| "outbound WebSocket is closed".to_string())?,
        }
    }

    async fn read(&self, cancel: &Arc<AtomicBool>, deadline: Instant) -> Result<Value, String> {
        let commands = self.commands()?;
        let (reply, result) = oneshot::channel();
        commands
            .send(ClientWebSocketCommand::Read { reply })
            .map_err(|_| "outbound WebSocket is closed".to_string())?;
        tokio::select! {
            biased;
            _ = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
            result = result => result.map_err(|_| "outbound WebSocket is closed".to_string())?,
        }
    }

    async fn close(&self, cancel: &Arc<AtomicBool>, deadline: Instant) -> Result<(), String> {
        let commands = self.commands()?;
        let (reply, result) = oneshot::channel();
        commands
            .send(ClientWebSocketCommand::Close { reply })
            .map_err(|_| "outbound WebSocket is closed".to_string())?;
        tokio::select! {
            biased;
            _ = cancelled(cancel, deadline) => Err(interrupt_err(cancel, deadline)),
            result = result => result.map_err(|_| "outbound WebSocket is closed".to_string())?,
        }
    }

    fn commands(&self) -> Result<mpsc::UnboundedSender<ClientWebSocketCommand>, String> {
        self.state
            .lock()
            .expect("client websocket state")
            .commands
            .clone()
            .ok_or_else(|| "outbound WebSocket is not connected".into())
    }
}

async fn run_client_websocket(
    mut socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    mut commands: mpsc::UnboundedReceiver<ClientWebSocketCommand>,
    mut cleared: watch::Receiver<u64>,
    state: Arc<StdMutex<ClientWebSocketState>>,
    current_generation: Arc<AtomicU64>,
    generation: u64,
) {
    let mut pending_read: Option<oneshot::Sender<Result<Value, String>>> = None;
    loop {
        tokio::select! {
            biased;
            _ = cleared.changed() => break,
            command = commands.recv() => match command {
                Some(ClientWebSocketCommand::Read { reply }) => {
                    if pending_read.is_some() {
                        let _ = reply.send(Err("outbound WebSocket read is already pending".into()));
                    } else {
                        pending_read = Some(reply);
                    }
                }
                Some(ClientWebSocketCommand::Send { data, reply }) => {
                    let result = tokio::select! {
                        _ = cleared.changed() => Err("outbound WebSocket request ended".into()),
                        result = socket.send(Message::Text(data.into())) => result.map_err(|err| err.to_string()),
                    };
                    let _ = reply.send(result);
                }
                Some(ClientWebSocketCommand::Close { reply }) => {
                    let result = tokio::select! {
                        _ = cleared.changed() => Err("outbound WebSocket request ended".into()),
                        result = socket.close(None) => result.map_err(|err| err.to_string()),
                    };
                    if let Some(read) = pending_read.take() {
                        let _ = read.send(Ok(websocket_close_value(1000, "", result.is_ok())));
                    }
                    let _ = reply.send(result);
                    break;
                }
                None => break,
            },
            incoming = socket.next(), if pending_read.is_some() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let reply = pending_read.take().expect("pending websocket read");
                        let _ = reply.send(Ok(Value::Record(vec![
                            ("type".into(), Value::String("text".into())),
                            ("data".into(), Value::String(text.to_string())),
                        ])));
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let reply = pending_read.take().expect("pending websocket read");
                        let _ = reply.send(Ok(Value::Record(vec![
                            ("type".into(), Value::String("binary".into())),
                            ("data".into(), Value::Bytes(bytes.to_vec())),
                        ])));
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let (code, reason) = frame
                            .map(|frame| (u16::from(frame.code), frame.reason.to_string()))
                            .unwrap_or((1005, String::new()));
                        let reply = pending_read.take().expect("pending websocket read");
                        let _ = reply.send(Ok(websocket_close_value(code, &reason, true)));
                        break;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        let reply = pending_read.take().expect("pending websocket read");
                        let _ = reply.send(Err(error.to_string()));
                        break;
                    }
                    None => {
                        let reply = pending_read.take().expect("pending websocket read");
                        let _ = reply.send(Ok(websocket_close_value(1006, "", false)));
                        break;
                    }
                }
            }
        }
    }
    if let Some(reply) = pending_read {
        let _ = reply.send(Err("outbound WebSocket request ended".into()));
    }
    if current_generation.load(Ordering::SeqCst) == generation {
        state.lock().expect("client websocket state").commands = None;
    }
}

fn websocket_close_value(code: u16, reason: &str, was_clean: bool) -> Value {
    Value::Record(vec![
        ("type".into(), Value::String("close".into())),
        ("code".into(), Value::Number(code.into())),
        ("reason".into(), Value::String(reason.into())),
        ("wasClean".into(), Value::Bool(was_clean)),
    ])
}

#[derive(Clone)]
pub struct IoHandle {
    tx: UnboundedSender<IoWork>,
    next_id: Arc<AtomicU64>,
    request_id: Arc<AtomicU64>,
    operation_cancels: Arc<StdMutex<OperationRegistry>>,
    pub inbound: StreamSlot,
    pub outbound: StreamRegistry,
    pub ws_in: StreamSlot,
    pub ws_out: SendSlot,
    pub client_ws: ClientWebSocketSlot,
}

#[derive(Clone)]
struct OperationControl {
    request_id: u64,
    cancel: Arc<AtomicBool>,
    bytes: usize,
    completed: bool,
}

#[derive(Default)]
struct OperationRegistry {
    operations: HashMap<OpId, OperationControl>,
    bytes: usize,
}

/// Completion admission uses the same reservation as submission. A completed
/// operation still occupies its slot until the isolate consumes/discards it.
#[derive(Clone)]
pub struct IoCompletionSender {
    tx: std::sync::mpsc::Sender<IoCompletion>,
    operations: Arc<StdMutex<OperationRegistry>>,
}

impl IoCompletionSender {
    /// Execute an admitted bridge Sleep on the shared I/O runtime so it cannot
    /// block later timers or capability forwarding. Cancellation and completion
    /// retain the operation's existing budget reservation until consumption.
    pub fn spawn_sleep(
        &self,
        id: OpId,
        millis: u64,
        cancel: crate::IsolateCancel,
        deadline: Instant,
    ) {
        let operation_cancel = self
            .operations
            .lock()
            .expect("operation cancellation registry")
            .operations
            .get(&id)
            .filter(|operation| !operation.completed)
            .map(|operation| operation.cancel.clone());
        let Some(operation_cancel) = operation_cancel else {
            return;
        };
        let completions = self.clone();
        let isolate_cancel = cancel.flag();
        io_handle().spawn(async move {
            let result = tokio::select! {
                biased;
                () = cancellation_flagged(&isolate_cancel) => Err(InterruptReason::Cancelled),
                result = wait(Duration::from_millis(millis), &operation_cancel, deadline) => result,
            };
            let _ = completions.send(IoCompletion { id, result: result.map_err(io_err) });
        });
    }

    pub fn send(
        &self,
        mut completion: IoCompletion,
    ) -> Result<(), std::sync::mpsc::SendError<IoCompletion>> {
        let mut registry = self.operations.lock().expect("operation cancellation registry");
        if let Some(operation) = registry.operations.get(&completion.id) {
            if operation.completed {
                return Ok(());
            }
            let old = operation.bytes;
            let bytes = size_of::<IoCompletion>()
                + match &completion.result {
                    Ok(value) => value_bytes(value),
                    Err(error) => error.capacity(),
                };
            let extra = bytes.saturating_sub(old);
            if extra > MAX_PENDING_IO_BYTES.saturating_sub(registry.bytes) {
                completion.result = Err(IO_BUDGET_ERROR.into());
            } else {
                registry.bytes += extra;
                registry.operations.get_mut(&completion.id).expect("registered operation").bytes +=
                    extra;
            }
            registry.operations.get_mut(&completion.id).expect("registered operation").completed =
                true;
        } else {
            // Ignore late/unknown bridge replies instead of accumulating
            // completions which no isolate operation can consume.
            return Ok(());
        }
        drop(registry);
        self.tx.send(completion)
    }
}

fn value_bytes(value: &Value) -> usize {
    match value {
        Value::String(value) => value.capacity(),
        Value::Bytes(value) => value.capacity(),
        Value::Array(values) => {
            values.capacity() * size_of::<Value>() + values.iter().map(value_bytes).sum::<usize>()
        }
        Value::Record(values) => {
            values.capacity() * size_of::<(String, Value)>()
                + values
                    .iter()
                    .map(|(key, value)| key.capacity() + value_bytes(value))
                    .sum::<usize>()
        }
        _ => 0,
    }
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

    pub fn check_capacity(&self, bytes: usize) -> Result<(), &'static str> {
        let registry = self.operation_cancels.lock().expect("operation cancellation registry");
        if registry.operations.len() >= MAX_PENDING_IO_OPS
            || bytes > MAX_PENDING_IO_BYTES.saturating_sub(registry.bytes)
        {
            return Err(IO_BUDGET_ERROR);
        }
        Ok(())
    }

    pub fn submit(&self, request: impl FnOnce(OpId) -> IoRequest) -> Result<OpId, &'static str> {
        self.check_capacity(0)?;
        let id = OpId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let request_id = self.request_id.load(Ordering::Relaxed);
        let request = request(id);
        let bytes = request.retained_bytes();
        let mut registry = self.operation_cancels.lock().expect("operation cancellation registry");
        if registry.operations.len() >= MAX_PENDING_IO_OPS
            || bytes > MAX_PENDING_IO_BYTES.saturating_sub(registry.bytes)
        {
            return Err(IO_BUDGET_ERROR);
        }
        registry.operations.insert(
            id,
            OperationControl {
                request_id,
                cancel: Arc::new(AtomicBool::new(false)),
                bytes,
                completed: false,
            },
        );
        registry.bytes += bytes;
        drop(registry);
        if self.tx.send(IoWork { request, request_id }).is_err() {
            self.finish(id);
            return Err("host I/O reactor stopped");
        }
        Ok(id)
    }

    pub fn cancel(&self, id: OpId) -> bool {
        let cancels = self.operation_cancels.lock().expect("operation cancellation registry");
        let Some(operation) = cancels.operations.get(&id) else {
            return false;
        };
        operation.cancel.store(true, Ordering::SeqCst);
        true
    }

    /// Cancel every operation owned by one completed HTTP request.
    ///
    /// Entries remain registered until their completion is consumed so work
    /// that has already been queued still observes the cancellation flag.
    pub fn cancel_request(&self, request_id: u64) -> Vec<OpId> {
        let operations = self.operation_cancels.lock().expect("operation cancellation registry");
        operations
            .operations
            .iter()
            .filter(|(_, operation)| operation.request_id == request_id)
            .map(|(id, operation)| {
                operation.cancel.store(true, Ordering::SeqCst);
                *id
            })
            .collect()
    }

    pub fn finish(&self, id: OpId) {
        let mut registry = self.operation_cancels.lock().expect("operation cancellation registry");
        if let Some(operation) = registry.operations.remove(&id) {
            registry.bytes -= operation.bytes;
        }
    }
}

pub struct Reactor {
    pub io: IoHandle,
    pub completions: std::sync::mpsc::Receiver<IoCompletion>,
}

pub fn spawn_reactor(cancel: Arc<AtomicBool>, deadline: Instant) -> Reactor {
    let inbound = StreamSlot::new();
    let outbound = StreamRegistry::new();
    let ws_in = StreamSlot::new();
    let ws_out = SendSlot::new();
    let client_ws = ClientWebSocketSlot::default();
    let operation_cancels = Arc::new(StdMutex::new(OperationRegistry::default()));
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let done_tx = IoCompletionSender { tx: done_tx, operations: operation_cancels.clone() };
    let inbound_task = inbound.clone();
    let outbound_task = outbound.clone();
    let ws_in_task = ws_in.clone();
    let ws_out_task = ws_out.clone();
    let client_ws_task = client_ws.clone();
    let operation_cancels_task = operation_cancels.clone();
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
                client_ws: client_ws_task,
            },
            operation_cancels_task,
        )
        .await;
    });

    Reactor {
        io: IoHandle {
            tx: req_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            request_id: Arc::new(AtomicU64::new(0)),
            operation_cancels,
            inbound,
            outbound,
            ws_in,
            ws_out,
            client_ws,
        },
        completions: done_rx,
    }
}

pub fn spawn_reactor_until_cancel(cancel: Arc<AtomicBool>) -> Reactor {
    spawn_reactor(cancel, Instant::now() + Duration::from_secs(60 * 60 * 24 * 365))
}

/// Split I/O so a process-isolated worker can proxy host calls over IPC.
pub fn open_bridge() -> (Reactor, UnboundedReceiver<IoWork>, IoCompletionSender) {
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let operation_cancels = Arc::new(StdMutex::new(OperationRegistry::default()));
    let done_tx = IoCompletionSender { tx: done_tx, operations: operation_cancels.clone() };
    (
        Reactor {
            io: IoHandle {
                tx: req_tx,
                next_id: Arc::new(AtomicU64::new(1)),
                request_id: Arc::new(AtomicU64::new(0)),
                operation_cancels,
                inbound: StreamSlot::new(),
                outbound: StreamRegistry::new(),
                ws_in: StreamSlot::new(),
                ws_out: SendSlot::new(),
                client_ws: ClientWebSocketSlot::default(),
            },
            completions: done_rx,
        },
        req_rx,
        done_tx,
    )
}

struct IoSlots {
    inbound: StreamSlot,
    outbound: StreamRegistry,
    ws_in: StreamSlot,
    ws_out: SendSlot,
    client_ws: ClientWebSocketSlot,
}

async fn run_reactor(
    mut requests: UnboundedReceiver<IoWork>,
    completions: IoCompletionSender,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    slots: IoSlots,
    operation_cancels: Arc<StdMutex<OperationRegistry>>,
) {
    while let Some(work) = requests.recv().await {
        let operation_id = work.request.id();
        let operation_cancel = operation_cancels
            .lock()
            .expect("operation cancellation registry")
            .operations
            .get(&operation_id)
            .map(|operation| operation.cancel.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let completions = completions.clone();
        let isolate_cancel = cancel.clone();
        let slots = IoSlots {
            inbound: slots.inbound.clone(),
            outbound: slots.outbound.clone(),
            ws_in: slots.ws_in.clone(),
            ws_out: slots.ws_out.clone(),
            client_ws: slots.client_ws.clone(),
        };
        tokio::spawn(async move {
            let execution =
                execute(work.request, work.request_id, operation_cancel.clone(), deadline, slots);
            tokio::pin!(execution);
            let completion = tokio::select! {
                completion = &mut execution => completion,
                () = cancellation_flagged(&isolate_cancel) => {
                    operation_cancel.store(true, Ordering::SeqCst);
                    execution.await
                }
            };
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
    if !matches!(request, IoRequest::ResponseWrite { .. } | IoRequest::ResponseClosed { .. })
        && let Err(error) = crate::trust::require(request.capability())
    {
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
        IoRequest::ResponseClosed { id, tx, stop } => IoCompletion {
            id,
            result: tokio::select! {
                biased;
                () = cancelled(&cancel, deadline) => Err(interrupt_err(&cancel, deadline)),
                _ = stop => Err("response stream finished".into()),
                () = tx.closed() => Ok(Value::Null),
            },
        },
        IoRequest::ResponseWrite { id, tx, bytes } => IoCompletion {
            id,
            result: tokio::select! {
                biased;
                () = cancelled(&cancel, deadline) => Err(interrupt_err(&cancel, deadline)),
                result = tx.send(bytes) => result.map(|()| Value::Null).map_err(|_| "response consumer closed".into()),
            },
        },
        IoRequest::ReadBody { id } => IoCompletion {
            id,
            result: slots
                .inbound
                .read(&cancel, deadline)
                .await
                .map(|chunk| chunk.map_or(Value::Null, Value::Bytes)),
        },
        IoRequest::HttpRead { id, body_id } => IoCompletion {
            id,
            result: read_http_chunk_interruptible(&slots.outbound, body_id, &cancel, deadline)
                .await,
        },
        IoRequest::HttpGet { id, request } => IoCompletion {
            id,
            result: outbound_fetch(request, cancel, deadline, slots.outbound).await,
        },
        IoRequest::WsRead { id } => {
            IoCompletion { id, result: read_chunk(&slots.ws_in, &cancel, deadline).await }
        }
        IoRequest::WsSend { id, data } => IoCompletion {
            id,
            result: slots
                .ws_out
                .send(data.into_bytes(), &cancel, deadline)
                .await
                .map(|()| Value::Null),
        },
        IoRequest::WsClose { id } => {
            slots.ws_out.close().await;
            IoCompletion { id, result: Ok(Value::Null) }
        }
        IoRequest::WsConnect { id, url } => IoCompletion {
            id,
            result: slots.client_ws.connect(url, &cancel, deadline).await.map(|()| Value::Null),
        },
        IoRequest::WsClientRead { id } => {
            IoCompletion { id, result: slots.client_ws.read(&cancel, deadline).await }
        }
        IoRequest::WsClientSend { id, data } => IoCompletion {
            id,
            result: slots.client_ws.send(data, &cancel, deadline).await.map(|()| Value::Null),
        },
        IoRequest::WsClientClose { id } => IoCompletion {
            id,
            result: slots.client_ws.close(&cancel, deadline).await.map(|()| Value::Null),
        },
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
        IoRequest::RedisGet { id, key } => IoCompletion {
            id,
            result: redis_op(tysel_cap_redis::get(&key), cancel, deadline).await,
        },
        IoRequest::RedisSet { id, key, value, ttl_seconds } => IoCompletion {
            id,
            result: redis_op(tysel_cap_redis::set(&key, &value, ttl_seconds), cancel, deadline)
                .await,
        },
        IoRequest::RedisDel { id, keys_json } => IoCompletion {
            id,
            result: match serde_json::from_str::<Vec<String>>(&keys_json) {
                Ok(keys) => redis_op(tysel_cap_redis::del(&keys), cancel, deadline).await,
                Err(_) => Err("redis keys must be a JSON string array".into()),
            },
        },
        IoRequest::RedisExists { id, key } => IoCompletion {
            id,
            result: redis_op(tysel_cap_redis::exists(&key), cancel, deadline).await,
        },
        IoRequest::RedisExpire { id, key, ttl_seconds } => IoCompletion {
            id,
            result: redis_op(tysel_cap_redis::expire(&key, ttl_seconds), cancel, deadline).await,
        },
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

async fn redis_op<F>(future: F, cancel: Arc<AtomicBool>, deadline: Instant) -> Result<Value, String>
where
    F: std::future::Future<Output = Result<Value, String>>,
{
    tokio::select! {
        biased;
        result = future => result,
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
        _ = cancelled(&cancel, deadline) => {
            let _ = task.await;
            Err(interrupt_err(&cancel, deadline))
        },
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

async fn read_chunk(
    slot: &StreamSlot,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Value, String> {
    match slot.read(cancel, deadline).await {
        Ok(Some(bytes)) => Ok(Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        Ok(None) => Ok(Value::Null),
        Err(err) => Err(err),
    }
}

async fn read_http_chunk_interruptible(
    streams: &StreamRegistry,
    body_id: u64,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Value, String> {
    streams
        .read(body_id, cancel, deadline)
        .await
        .map(|chunk| chunk.map_or(Value::Null, Value::Bytes))
}

const MAX_REDIRECTS: u8 = 20;
const MAX_OUTBOUND_BODY: usize = 16 * 1024 * 1024;

struct Hop {
    response: hyper::Response<Incoming>,
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
}

async fn outbound_fetch(
    request: FetchRequest,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    outbound: StreamRegistry,
) -> Result<Value, String> {
    let FetchRequest { method, url, headers_json, body, redirect } = request;
    let mut method = normalize_method(&method)?;
    let mut headers = crate::fetch_policy::expand_headers_json(&headers_json)?;
    let mut body = request_body(&method, body)?;
    let mut url = url;
    for _ in 0..=MAX_REDIRECTS {
        let hop =
            fetch_hop(&method, &url, &headers.headers, body.clone(), &cancel, deadline).await?;
        let status = hop.response.status();
        let is_redirect = matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308);
        if is_redirect && redirect == RedirectMode::Error {
            return Err("HTTP redirect rejected by redirect mode".into());
        }
        if is_redirect
            && redirect == RedirectMode::Follow
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
        if cancel.load(Ordering::SeqCst) {
            return Err(interrupt_err(&cancel, deadline));
        }
        let body_id = outbound.install(rx).await;
        if cancel.load(Ordering::SeqCst) {
            outbound.clear_async(body_id).await;
            return Err(interrupt_err(&cancel, deadline));
        }
        io_handle().spawn(async move {
            let _keep_alive = hop.sender;
            pump_http_body(hop.response.into_body(), tx, cancel, deadline).await;
        });
        return Ok(Value::Record(vec![
            ("status".into(), Value::Number(f64::from(code))),
            ("headers".into(), Value::String(headers_json)),
            ("bodyId".into(), Value::Number(body_id as f64)),
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

fn request_body(method: &str, body: Bytes) -> Result<Bytes, String> {
    if method == "GET" || method == "HEAD" {
        return Ok(Bytes::new());
    }
    if body.len() > MAX_OUTBOUND_BODY {
        return Err(format!("request body exceeds {MAX_OUTBOUND_BODY} bytes"));
    }
    Ok(body)
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

async fn cancellation_flagged(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(1)).await;
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
            () = tx.closed() => return,
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

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn bridge_sleep_observes_operation_cancel_isolate_cancel_and_deadline() {
        for scenario in ["operation cancel", "isolate cancel", "deadline"] {
            let (reactor, mut requests, complete) = open_bridge();
            let id = reactor.io.submit(|id| IoRequest::Sleep { id, millis: 60_000 }).unwrap();
            let work = requests.try_recv().unwrap();
            assert_eq!(work.request.id(), id);
            let cancel = crate::IsolateCancel::new();
            let deadline = if scenario == "deadline" {
                Instant::now()
            } else {
                Instant::now() + Duration::from_secs(60)
            };
            if scenario == "operation cancel" {
                reactor.io.cancel(id);
            }
            complete.spawn_sleep(id, 60_000, cancel.clone(), deadline);
            if scenario == "isolate cancel" {
                cancel.cancel();
            }
            let result = reactor.completions.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(result.id, id);
            let expected = if scenario == "deadline" { "Timeout" } else { "Cancelled" };
            assert_eq!(result.result, Err(expected.into()), "{scenario}");
            reactor.io.finish(id);
            let registry = reactor.io.operation_cancels.lock().unwrap();
            assert!(registry.operations.is_empty());
            assert_eq!(registry.bytes, 0);
        }
    }

    #[test]
    fn io_slots_cover_queued_running_and_unconsumed_completions() {
        let (reactor, mut requests, complete) = open_bridge();
        for _ in 0..MAX_PENDING_IO_OPS {
            reactor.io.submit(|id| IoRequest::Sleep { id, millis: 1000 }).unwrap();
        }
        assert_eq!(
            reactor.io.submit(|id| IoRequest::Sleep { id, millis: 1 }),
            Err(IO_BUDGET_ERROR)
        );
        while let Ok(work) = requests.try_recv() {
            complete.send(IoCompletion { id: work.request.id(), result: Ok(Value::Null) }).unwrap();
            complete.send(IoCompletion { id: work.request.id(), result: Ok(Value::Null) }).unwrap();
        }
        assert_eq!(
            reactor.io.submit(|id| IoRequest::Sleep { id, millis: 1 }),
            Err(IO_BUDGET_ERROR)
        );
        let mut consumed = 0;
        while let Ok(completion) = reactor.completions.try_recv() {
            reactor.io.finish(completion.id);
            consumed += 1;
        }
        assert_eq!(consumed, MAX_PENDING_IO_OPS);
        let registry = reactor.io.operation_cancels.lock().unwrap();
        assert!(registry.operations.is_empty());
        assert_eq!(registry.bytes, 0);
        drop(registry);
        assert!(reactor.io.submit(|id| IoRequest::Sleep { id, millis: 1 }).is_ok());
    }

    #[test]
    fn io_byte_budget_includes_results_and_recovers_after_cancellation() {
        let (reactor, mut requests, complete) = open_bridge();
        reactor.io.bind_request(42);
        let first = reactor
            .io
            .submit(|id| IoRequest::Echo { id, value: "x".repeat(MAX_PENDING_IO_BYTES / 2) })
            .unwrap();
        assert_eq!(
            reactor
                .io
                .submit(|id| IoRequest::Echo { id, value: "x".repeat(MAX_PENDING_IO_BYTES / 2) }),
            Err(IO_BUDGET_ERROR)
        );
        let second = reactor.io.submit(|id| IoRequest::Sleep { id, millis: 1 }).unwrap();
        complete
            .send(IoCompletion {
                id: second,
                result: Ok(Value::String("x".repeat(MAX_PENDING_IO_BYTES / 2))),
            })
            .unwrap();
        let result = reactor.completions.try_recv().unwrap();
        assert_eq!(result.result, Err(IO_BUDGET_ERROR.into()));
        reactor.io.finish(result.id);
        let canceled = reactor.io.cancel_request(42);
        assert_eq!(canceled, vec![first]);
        while requests.try_recv().is_ok() {}
        complete.send(IoCompletion { id: first, result: Err("canceled".into()) }).unwrap();
        reactor.io.finish(reactor.completions.try_recv().unwrap().id);
        let registry = reactor.io.operation_cancels.lock().unwrap();
        assert!(registry.operations.is_empty());
        assert_eq!(registry.bytes, 0);
        drop(registry);
        // Large results from small requests consume the same aggregate budget.
        let id = reactor.io.submit(|id| IoRequest::FsRead { id, path: "test".into() }).unwrap();
        complete
            .send(IoCompletion {
                id,
                result: Ok(Value::Bytes(vec![0; MAX_PENDING_IO_BYTES - 1024])),
            })
            .unwrap();
        assert_eq!(
            reactor.io.submit(|id| IoRequest::Echo { id, value: "x".repeat(2048) }),
            Err(IO_BUDGET_ERROR)
        );
        reactor.io.finish(reactor.completions.try_recv().unwrap().id);
        assert!(reactor.io.submit(|id| IoRequest::Echo { id, value: "ok".into() }).is_ok());
    }

    fn active_operation() -> (Arc<AtomicBool>, Instant) {
        (Arc::new(AtomicBool::new(false)), Instant::now() + Duration::from_secs(1))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clear_all_invalidates_an_in_flight_read() {
        let streams = StreamRegistry::new();
        let (tx, rx) = mpsc::channel(1);
        let body_id = streams.install(rx).await;
        let read_streams = streams.clone();
        let (cancel, deadline) = active_operation();
        let read = tokio::spawn(async move { read_streams.read(body_id, &cancel, deadline).await });
        loop {
            if streams.inner.lock().await.in_flight.contains(&body_id) {
                break;
            }
            tokio::task::yield_now().await;
        }

        tokio::task::block_in_place(|| streams.clear_all());
        tx.send(Ok(b"late".to_vec())).await.expect("late chunk");
        let error = read.await.expect("join").expect_err("cleared read");
        assert!(error.contains("Cancelled"), "unexpected error: {error}");
        let state = streams.inner.lock().await;
        assert!(state.streams.is_empty());
        assert!(state.in_flight.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clear_invalidates_only_the_selected_in_flight_read() {
        let streams = StreamRegistry::new();
        let (first_tx, first_rx) = mpsc::channel(1);
        let (second_tx, second_rx) = mpsc::channel(1);
        let first_id = streams.install(first_rx).await;
        let second_id = streams.install(second_rx).await;
        let read_streams = streams.clone();
        let (cancel, deadline) = active_operation();
        let first_read =
            tokio::spawn(async move { read_streams.read(first_id, &cancel, deadline).await });
        loop {
            if streams.inner.lock().await.in_flight.contains(&first_id) {
                break;
            }
            tokio::task::yield_now().await;
        }

        tokio::task::block_in_place(|| streams.clear(first_id));
        first_tx.send(Ok(b"late".to_vec())).await.expect("late first chunk");
        second_tx.send(Ok(b"kept".to_vec())).await.expect("second chunk");
        let error = first_read.await.expect("join").expect_err("cleared read");
        assert!(error.contains("Cancelled"), "unexpected error: {error}");
        let (cancel, deadline) = active_operation();
        assert_eq!(
            streams.read(second_id, &cancel, deadline).await.expect("second read"),
            Some(b"kept".to_vec())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_a_read_cleans_its_in_flight_state() {
        let streams = StreamRegistry::new();
        let (_tx, rx) = mpsc::channel(1);
        let body_id = streams.install(rx).await;
        let read_streams = streams.clone();
        let (cancel, deadline) = active_operation();
        let read_cancel = cancel.clone();
        let read =
            tokio::spawn(async move { read_streams.read(body_id, &read_cancel, deadline).await });
        loop {
            if streams.inner.lock().await.in_flight.contains(&body_id) {
                break;
            }
            tokio::task::yield_now().await;
        }

        tokio::task::block_in_place(|| streams.clear(body_id));
        cancel.store(true, Ordering::SeqCst);
        let error = read.await.expect("join").expect_err("cancelled read");
        assert!(error.contains("Cancelled"), "unexpected error: {error}");
        let state = streams.inner.lock().await;
        assert!(state.streams.is_empty());
        assert!(state.in_flight.is_empty());
        assert!(state.cancelled.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_a_backpressured_websocket_send_releases_the_sender() {
        let slot = SendSlot::new();
        let (tx, _rx) = mpsc::channel(1);
        tokio::task::block_in_place(|| slot.install(tx));
        let (cancel, deadline) = active_operation();
        slot.send(b"first".to_vec(), &cancel, deadline).await.expect("fill channel");
        let send_slot = slot.clone();
        let send_cancel = cancel.clone();
        let send = tokio::spawn(async move {
            send_slot.send(b"blocked".to_vec(), &send_cancel, deadline).await
        });
        tokio::task::yield_now().await;

        cancel.store(true, Ordering::SeqCst);
        let error = send.await.expect("join").expect_err("cancelled send");
        assert!(error.contains("Cancelled"), "unexpected error: {error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_blocking_work_waits_for_it_to_quiesce() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let side_effect = Arc::new(AtomicBool::new(false));
        let work_effect = side_effect.clone();
        let work_cancel = cancel.clone();
        let work = tokio::spawn(async move {
            run_blocking(work_cancel, Instant::now() + Duration::from_secs(1), move || {
                started_tx.send(()).expect("started");
                release_rx.recv().expect("released");
                work_effect.store(true, Ordering::SeqCst);
                Ok::<_, String>(())
            })
            .await
        });
        tokio::task::block_in_place(|| {
            started_rx.recv_timeout(Duration::from_secs(1)).expect("blocking work started")
        });

        cancel.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!work.is_finished(), "cancellation detached blocking work");
        release_tx.send(()).expect("release work");
        let error = work.await.expect("join").expect_err("cancelled work");
        assert!(error.contains("Cancelled"), "unexpected error: {error}");
        assert!(side_effect.load(Ordering::SeqCst));
    }
}
