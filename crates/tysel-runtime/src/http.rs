use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use std::future::Future;
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tysel_engine::{EngineError, HttpHead, HttpRequest, InterruptReason, IsolateConfig};
use tysel_engine_qjs::{
    IncomingHttp, IsolatePool, ModuleMetadata, OutgoingHttpBody, STREAM_WINDOW,
};
use tysel_isolate::{IsolatedHttpPool, MAX_ISOLATED_HTTP_BODY, locate_worker};
use tysel_package::{
    SourceMap, default_max_in_flight, default_max_request_bytes, default_max_response_bytes,
};

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hyper: {0}")]
    Hyper(String),
    #[error("request body exceeds {0} bytes")]
    BodyTooLarge(usize),
    #[error("response body exceeds {0} bytes")]
    ResponseTooLarge(usize),
}

#[derive(Clone)]
pub enum AppIsolate {
    Trusted(Arc<IsolatePool>),
    Isolated(Arc<IsolatedHttpPool>),
}

impl From<Arc<IsolatePool>> for AppIsolate {
    fn from(pool: Arc<IsolatePool>) -> Self {
        Self::Trusted(pool)
    }
}

impl AppIsolate {
    async fn dispatch_incoming(
        &self,
        request: IncomingHttp,
        deadline: Instant,
    ) -> Result<(tysel_engine::HttpHead, OutgoingHttpBody), EngineError> {
        match self {
            Self::Trusted(pool) => pool.dispatch_response_until(request, deadline).await,
            Self::Isolated(pool) => tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                dispatch_isolated(pool.clone(), request, deadline),
            )
            .await
            .map_err(|_| EngineError::Interrupted(InterruptReason::Timeout))?,
        }
    }

    pub fn request_timeout(&self) -> Duration {
        match self {
            Self::Trusted(pool) => pool.request_timeout(),
            Self::Isolated(pool) => pool.request_timeout(),
        }
    }
}

pub fn spawn_app_isolate(
    execution_profile: &str,
    workers: u32,
    source: &str,
    config: IsolateConfig,
    secret_names: Vec<String>,
) -> Result<AppIsolate, EngineError> {
    spawn_app_isolate_with_metadata(execution_profile, workers, source, config, secret_names)
        .map(|(isolate, _)| isolate)
}

pub fn spawn_app_isolate_with_metadata(
    execution_profile: &str,
    workers: u32,
    source: &str,
    config: IsolateConfig,
    secret_names: Vec<String>,
) -> Result<(AppIsolate, Option<ModuleMetadata>), EngineError> {
    if execution_profile.eq_ignore_ascii_case("isolated") {
        let worker = locate_worker().map_err(|err| EngineError::Isolate(err.to_string()))?;
        let pool = IsolatedHttpPool::spawn_from_config(worker, source, config, secret_names)
            .map_err(|err| EngineError::Isolate(err.to_string()))?;
        Ok((AppIsolate::Isolated(Arc::new(pool)), None))
    } else {
        let (pool, metadata) =
            IsolatePool::spawn_with_metadata(workers.max(1) as usize, source, config)?;
        Ok((AppIsolate::Trusted(Arc::new(pool)), Some(metadata)))
    }
}

async fn dispatch_isolated(
    pool: Arc<IsolatedHttpPool>,
    request: IncomingHttp,
    deadline: Instant,
) -> Result<(HttpHead, OutgoingHttpBody), EngineError> {
    if request.ws_in.is_some() || request.ws_out.is_some() {
        return Err(EngineError::Isolate(
            "websocket is not available in the isolated profile".into(),
        ));
    }
    let mut body = Vec::new();
    let mut inbound = request.body;
    while let Some(chunk) = inbound.recv().await {
        let chunk = chunk.map_err(|err| {
            if err.contains("exceeds") {
                EngineError::BodyTooLarge
            } else {
                EngineError::Isolate(err)
            }
        })?;
        body.extend(chunk);
        if body.len() > MAX_ISOLATED_HTTP_BODY {
            return Err(EngineError::BodyTooLarge);
        }
    }
    struct CancelOnDrop(Arc<AtomicBool>);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancelled.clone());
    let result = tokio::task::spawn_blocking(move || {
        pool.dispatch_sync_cancellable(
            HttpRequest {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body,
                request_id: request.request_id,
            },
            deadline,
            &cancelled,
        )
    })
    .await
    .map_err(|err| EngineError::Isolate(err.to_string()))?;
    let (head, bytes) = result.map_err(|err| match err {
        tysel_isolate::IsolateError::Engine(error) => error,
        other => EngineError::Isolate(other.to_string()),
    })?;
    Ok((head, OutgoingHttpBody::Buffered(bytes)))
}

#[derive(Clone)]
pub struct SharedPool {
    inner: Arc<RwLock<PoolState>>,
    shutdown: Option<HttpShutdown>,
}

/// Cooperative connection draining. One watcher per connection, never per chunk.
#[derive(Clone)]
pub struct HttpShutdown(Arc<HttpShutdownState>);

struct HttpShutdownState {
    phase: watch::Sender<u8>,
    active: AtomicUsize,
    idle: Notify,
}

impl Default for HttpShutdown {
    fn default() -> Self {
        let (phase, _) = watch::channel(0);
        Self(Arc::new(HttpShutdownState {
            phase,
            active: AtomicUsize::new(0),
            idle: Notify::new(),
        }))
    }
}

impl HttpShutdown {
    pub async fn drain(&self, grace: Duration) {
        self.cancel();
        if tokio::time::timeout(grace, self.idle()).await.is_err() {
            self.0.phase.send_replace(2);
            self.idle().await;
        }
    }

    pub fn cancel(&self) {
        self.0.phase.send_if_modified(|phase| {
            if *phase == 0 {
                *phase = 1;
                true
            } else {
                false
            }
        });
    }

    async fn notified(&self, phase: u8) {
        let mut rx = self.0.phase.subscribe();
        while *rx.borrow_and_update() < phase {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    fn track(&self) -> ConnectionGuard {
        self.0.active.fetch_add(1, Ordering::AcqRel);
        ConnectionGuard(self.clone())
    }

    async fn idle(&self) {
        loop {
            let notified = self.0.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct ConnectionGuard(HttpShutdown);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if self.0.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.0.idle.notify_waiters();
        }
    }
}

struct PoolState {
    isolate: AppIsolate,
    max_request_bytes: usize,
    max_response_bytes: usize,
    max_in_flight: u32,
    admission: Arc<Admission>,
    websocket: bool,
    http1: bool,
    http2: bool,
    source_map: Option<Arc<SourceMap>>,
    diagnostic_sink: Option<RuntimeDiagnosticSink>,
}

struct PoolSnapshot {
    isolate: AppIsolate,
    limits: HttpLimits,
    websocket: bool,
    admission: Arc<Admission>,
    diagnostics: RuntimeDiagnostics,
}

struct RuntimeDiagnostics {
    shutdown: Option<HttpShutdown>,
    source_map: Option<Arc<SourceMap>>,
    sink: Option<RuntimeDiagnosticSink>,
}

pub type RuntimeDiagnosticSink = Arc<dyn Fn(RuntimeDiagnostic) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub request_id: u64,
    pub source: Option<RuntimeDiagnosticSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnosticSource {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy)]
pub struct HttpLimits {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_in_flight: u32,
}

struct Admission {
    active: AtomicU32,
    limit: AtomicU32,
}

impl Admission {
    fn new(limit: u32) -> Self {
        Self { active: AtomicU32::new(0), limit: AtomicU32::new(limit) }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<AdmissionPermit> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            let limit = self.limit.load(Ordering::Acquire);
            if active >= limit {
                return None;
            }
            match self.active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(AdmissionPermit { admission: self.clone() }),
                Err(current) => active = current,
            }
        }
    }

    fn set_limit(&self, limit: u32) {
        self.limit.store(limit, Ordering::Release);
    }

    #[cfg(test)]
    fn available(&self) -> u32 {
        self.limit.load(Ordering::Acquire).saturating_sub(self.active.load(Ordering::Acquire))
    }
}

struct AdmissionPermit {
    admission: Arc<Admission>,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let previous = self.admission.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "admission permit count underflow");
    }
}

impl SharedPool {
    pub fn with_shutdown(mut self, shutdown: HttpShutdown) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    pub fn new(pool: impl Into<AppIsolate>, max_request_bytes: usize) -> Self {
        Self::with_websocket(pool, max_request_bytes, false)
    }

    pub fn with_websocket(
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
    ) -> Self {
        Self::with_debug_info(pool, max_request_bytes, websocket, None)
    }

    pub fn with_debug_info(
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
        source_map: Option<Arc<SourceMap>>,
    ) -> Self {
        Self::with_server_options(pool, max_request_bytes, websocket, true, false, source_map)
    }

    pub fn with_server_options(
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
        http1: bool,
        http2: bool,
        source_map: Option<Arc<SourceMap>>,
    ) -> Self {
        Self::with_server_limits(
            pool,
            max_request_bytes,
            default_max_in_flight(),
            websocket,
            http1,
            http2,
            source_map,
        )
    }

    pub fn with_server_limits(
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        max_in_flight: u32,
        websocket: bool,
        http1: bool,
        http2: bool,
        source_map: Option<Arc<SourceMap>>,
    ) -> Self {
        Self::with_http_limits(
            pool,
            HttpLimits {
                max_request_bytes,
                max_response_bytes: default_max_response_bytes(),
                max_in_flight,
            },
            websocket,
            http1,
            http2,
            source_map,
        )
    }

    pub fn with_http_limits(
        pool: impl Into<AppIsolate>,
        limits: HttpLimits,
        websocket: bool,
        http1: bool,
        http2: bool,
        source_map: Option<Arc<SourceMap>>,
    ) -> Self {
        Self {
            shutdown: None,
            inner: Arc::new(RwLock::new(PoolState {
                isolate: pool.into(),
                max_request_bytes: limits.max_request_bytes,
                max_response_bytes: limits.max_response_bytes,
                max_in_flight: limits.max_in_flight,
                admission: Arc::new(Admission::new(limits.max_in_flight)),
                websocket,
                http1,
                http2,
                source_map,
                diagnostic_sink: None,
            })),
        }
    }

    pub fn set_diagnostic_sink(&self, diagnostic_sink: Option<RuntimeDiagnosticSink>) {
        self.inner.write().expect("pool lock").diagnostic_sink = diagnostic_sink;
    }

    pub fn replace(&self, pool: impl Into<AppIsolate>, max_request_bytes: usize) {
        self.replace_with(pool, max_request_bytes, self.websocket());
    }

    pub fn replace_with(
        &self,
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
    ) {
        self.replace_with_debug_info(pool, max_request_bytes, websocket, None);
    }

    pub fn replace_with_debug_info(
        &self,
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
        source_map: Option<Arc<SourceMap>>,
    ) {
        self.replace_with_server_options(
            pool,
            max_request_bytes,
            websocket,
            true,
            false,
            source_map,
        );
    }

    pub fn replace_with_server_options(
        &self,
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
        http1: bool,
        http2: bool,
        source_map: Option<Arc<SourceMap>>,
    ) {
        let max_in_flight = self.max_in_flight();
        let max_response_bytes = self.max_response_bytes();
        self.replace_with_server_limits(
            pool,
            HttpLimits { max_request_bytes, max_response_bytes, max_in_flight },
            websocket,
            http1,
            http2,
            source_map,
        );
    }

    pub fn replace_with_server_limits(
        &self,
        pool: impl Into<AppIsolate>,
        limits: HttpLimits,
        websocket: bool,
        http1: bool,
        http2: bool,
        source_map: Option<Arc<SourceMap>>,
    ) {
        let previous = {
            let mut guard = self.inner.write().expect("pool lock");
            let admission = guard.admission.clone();
            let diagnostic_sink = guard.diagnostic_sink.clone();
            admission.set_limit(limits.max_in_flight);
            std::mem::replace(
                &mut *guard,
                PoolState {
                    isolate: pool.into(),
                    max_request_bytes: limits.max_request_bytes,
                    max_response_bytes: limits.max_response_bytes,
                    max_in_flight: limits.max_in_flight,
                    admission,
                    websocket,
                    http1,
                    http2,
                    source_map,
                    diagnostic_sink,
                },
            )
        };
        drop(previous);
    }

    pub fn current(&self) -> (AppIsolate, usize, bool, Option<Arc<SourceMap>>) {
        let guard = self.inner.read().expect("pool lock");
        (guard.isolate.clone(), guard.max_request_bytes, guard.websocket, guard.source_map.clone())
    }

    fn current_with_admission(&self) -> PoolSnapshot {
        let guard = self.inner.read().expect("pool lock");
        PoolSnapshot {
            isolate: guard.isolate.clone(),
            limits: HttpLimits {
                max_request_bytes: guard.max_request_bytes,
                max_response_bytes: guard.max_response_bytes,
                max_in_flight: guard.max_in_flight,
            },
            websocket: guard.websocket,
            admission: guard.admission.clone(),
            diagnostics: RuntimeDiagnostics {
                shutdown: self.shutdown.clone(),
                source_map: guard.source_map.clone(),
                sink: guard.diagnostic_sink.clone(),
            },
        }
    }

    pub fn max_in_flight(&self) -> u32 {
        self.inner.read().expect("pool lock").max_in_flight
    }

    pub fn max_response_bytes(&self) -> usize {
        self.inner.read().expect("pool lock").max_response_bytes
    }

    #[cfg(test)]
    pub(crate) fn available_admission_permits(&self) -> usize {
        self.inner.read().expect("pool lock").admission.available() as usize
    }

    pub fn websocket(&self) -> bool {
        self.inner.read().expect("pool lock").websocket
    }

    pub fn protocols(&self) -> (bool, bool) {
        let guard = self.inner.read().expect("pool lock");
        (guard.http1, guard.http2)
    }
}

pub async fn serve(
    listener: TcpListener,
    pool: impl Into<AppIsolate>,
    max_request_bytes: usize,
) -> Result<(), HttpError> {
    serve_with_websocket(listener, pool, max_request_bytes, false).await
}

pub async fn serve_with_websocket(
    listener: TcpListener,
    pool: impl Into<AppIsolate>,
    max_request_bytes: usize,
    websocket: bool,
) -> Result<(), HttpError> {
    serve_with_protocols(listener, pool, max_request_bytes, websocket, true, false).await
}

pub async fn serve_with_protocols(
    listener: TcpListener,
    pool: impl Into<AppIsolate>,
    max_request_bytes: usize,
    websocket: bool,
    http1: bool,
    http2: bool,
) -> Result<(), HttpError> {
    serve_with_limits(
        listener,
        pool,
        max_request_bytes,
        default_max_in_flight(),
        websocket,
        http1,
        http2,
    )
    .await
}

pub async fn serve_with_limits(
    listener: TcpListener,
    pool: impl Into<AppIsolate>,
    max_request_bytes: usize,
    max_in_flight: u32,
    websocket: bool,
    http1: bool,
    http2: bool,
) -> Result<(), HttpError> {
    serve_with_http_limits(
        listener,
        pool,
        HttpLimits {
            max_request_bytes,
            max_response_bytes: default_max_response_bytes(),
            max_in_flight,
        },
        websocket,
        http1,
        http2,
    )
    .await
}

pub async fn serve_with_http_limits(
    listener: TcpListener,
    pool: impl Into<AppIsolate>,
    limits: HttpLimits,
    websocket: bool,
    http1: bool,
    http2: bool,
) -> Result<(), HttpError> {
    if !http1 && !http2 {
        return Err(HttpError::Hyper("at least one HTTP protocol must be enabled".into()));
    }
    let pool = SharedPool::with_http_limits(pool, limits, websocket, http1, http2, None);
    loop {
        let (stream, _peer) = listener.accept().await?;
        handle_stream(stream, pool.clone());
    }
}

/// Serve until cancellation, then drain admitted responses and close idle sockets.
/// The grace period is shared by every connection, including upgraded sockets.
pub async fn serve_with_shutdown(
    listener: TcpListener,
    pool: SharedPool,
    shutdown: HttpShutdown,
    grace: Duration,
) -> Result<(), HttpError> {
    let (http1, http2) = pool.protocols();
    if !http1 && !http2 {
        return Err(HttpError::Hyper("at least one HTTP protocol must be enabled".into()));
    }
    let pool = pool.with_shutdown(shutdown.clone());
    let result = loop {
        tokio::select! {
            biased;
            () = shutdown.notified(1) => break Ok(()),
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => handle_stream(stream, pool.clone()),
                Err(error) => break Err(HttpError::Io(error)),
            }
        }
    };
    drop(listener);
    shutdown.drain(grace).await;
    result
}

pub fn handle_stream(stream: tokio::net::TcpStream, pool: SharedPool) {
    // Small HTTP responses should not wait for Nagle/delayed-ACK interaction.
    // This applies to every accepted connection, independent of the handler or
    // workload, and mirrors the latency-oriented behavior of modern runtimes.
    let _ = stream.set_nodelay(true);
    let shutdown = pool.shutdown.clone();
    let guard = shutdown.as_ref().map(HttpShutdown::track);
    tokio::spawn(async move {
        let _guard = guard;
        let (http1, http2) = pool.protocols();
        let io = TokioIo::new(stream);
        let service = service_fn(move |request| {
            let pool = pool.clone();
            async move {
                let method = request.method().as_str().to_owned();
                let path = request.uri().path().to_owned();
                let started = Instant::now();
                let request_id = tysel_observability::next_request_id();
                let current = pool.current_with_admission();
                let response = match current.admission.try_acquire() {
                    Some(permit) => {
                        dispatch(
                            current.isolate,
                            request,
                            current.limits,
                            current.websocket,
                            request_id,
                            current.diagnostics,
                            permit,
                        )
                        .await
                    }
                    None => overloaded_response(request_id),
                };
                tysel_observability::log_http(
                    &method,
                    &path,
                    response.status().as_u16(),
                    started.elapsed(),
                    request_id,
                );
                Ok::<_, Infallible>(response)
            }
        });
        // Pin each protocol's native connection, retaining its existing fast path.
        macro_rules! drive {
            ($connection:expr) => {{
                let connection = $connection;
                tokio::pin!(connection);
                if let Some(shutdown) = &shutdown {
                    tokio::select! {
                        result = &mut connection => { let _ = result; return; }
                        () = shutdown.notified(1) => connection.as_mut().graceful_shutdown(),
                    }
                    tokio::select! {
                        result = &mut connection => { let _ = result; }
                        () = shutdown.notified(2) => {}
                    }
                } else {
                    let _ = connection.await;
                }
            }};
        }
        match (http1, http2) {
            (true, true) => {
                let builder = auto::Builder::new(TokioExecutor::new());
                drive!(builder.serve_connection_with_upgrades(io, service));
            }
            (false, true) => {
                let builder = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
                drive!(builder.serve_connection(io, service));
            }
            (true, false) => {
                let mut builder = hyper::server::conn::http1::Builder::new();
                drive!(builder.keep_alive(true).serve_connection(io, service).with_upgrades());
            }
            (false, false) => {}
        }
    });
}

pub async fn bind(addr: SocketAddr, pool: Arc<IsolatePool>) -> Result<SocketAddr, HttpError> {
    bind_with_request_limit(addr, pool, default_max_request_bytes()).await
}

pub async fn bind_with_request_limit(
    addr: SocketAddr,
    pool: Arc<IsolatePool>,
    max_request_bytes: usize,
) -> Result<SocketAddr, HttpError> {
    bind_with(addr, pool, max_request_bytes, false).await
}

pub async fn bind_with(
    addr: SocketAddr,
    pool: Arc<IsolatePool>,
    max_request_bytes: usize,
    websocket: bool,
) -> Result<SocketAddr, HttpError> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = serve_with_websocket(listener, pool, max_request_bytes, websocket).await;
    });
    Ok(local)
}

async fn dispatch(
    pool: AppIsolate,
    request: Request<Incoming>,
    limits: HttpLimits,
    websocket: bool,
    request_id: u64,
    diagnostics: RuntimeDiagnostics,
    permit: AdmissionPermit,
) -> Response<HttpBody> {
    let mut permit = Some(permit);
    let mut response = match dispatch_inner(
        pool,
        request,
        limits,
        websocket,
        request_id,
        &mut permit,
        diagnostics.shutdown.clone(),
    )
    .await
    {
        Ok(response) => response,
        Err(HttpError::Engine(EngineError::Interrupted(InterruptReason::Timeout))) => {
            json_error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "REQUEST_TIMEOUT",
                "request deadline exceeded",
                request_id,
            )
        }
        Err(HttpError::BodyTooLarge(limit)) => json_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "BODY_TOO_LARGE",
            &format!("request body exceeds {limit} bytes"),
            request_id,
        ),
        Err(HttpError::ResponseTooLarge(limit)) => json_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RESPONSE_TOO_LARGE",
            &format!("response body exceeds {limit} bytes"),
            request_id,
        ),
        Err(err) => {
            // Source maps are supplied only by the explicit development
            // diagnostics path. Packaged services must never echo an
            // arbitrary application or host exception to their clients.
            let expose_details = diagnostics.source_map.is_some();
            let message = err.to_string();
            let source = diagnostics
                .source_map
                .as_deref()
                .and_then(|source_map| source_map.first_original_position(&message))
                .map(|position| RuntimeDiagnosticSource {
                    file: position.source,
                    line: position.line,
                    column: position.column,
                });
            let message = diagnostics
                .source_map
                .as_deref()
                .map(|source_map| source_map.symbolicate_stack(&message))
                .unwrap_or(message);
            if let Some(diagnostic_sink) = diagnostics.sink.as_ref() {
                diagnostic_sink(RuntimeDiagnostic {
                    code: "RUNTIME_ERROR",
                    message: message.clone(),
                    request_id,
                    source,
                });
            }
            json_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "RUNTIME_ERROR",
                if expose_details { &message } else { "request execution failed" },
                request_id,
            )
        }
    };
    if let Some(permit) = permit {
        response.body_mut().hold_permit(permit);
    }
    response
}

fn json_error_response(
    status: StatusCode,
    code: &str,
    message: &str,
    request_id: u64,
) -> Response<HttpBody> {
    let body = serde_json::to_vec(&serde_json::json!({
        "error": {
            "code": code,
            "message": message,
            "requestId": format!("{request_id:016x}"),
        }
    }))
    .unwrap_or_else(|_| b"{\"error\":{\"code\":\"INTERNAL_ERROR\"}}".to_vec());
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(HttpBody::once(body))
        .unwrap_or_else(|_| Response::new(HttpBody::once(Vec::new())))
}

fn overloaded_response(request_id: u64) -> Response<HttpBody> {
    let mut response = json_error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "OVERLOADED",
        "maximum in-flight request limit reached",
        request_id,
    );
    response
        .headers_mut()
        .insert(hyper::header::RETRY_AFTER, hyper::header::HeaderValue::from_static("1"));
    response
}

async fn dispatch_inner(
    pool: AppIsolate,
    request: Request<Incoming>,
    limits: HttpLimits,
    websocket_enabled: bool,
    request_id: u64,
    permit: &mut Option<AdmissionPermit>,
    shutdown: Option<HttpShutdown>,
) -> Result<Response<HttpBody>, HttpError> {
    let deadline = Instant::now()
        .checked_add(pool.request_timeout())
        .ok_or_else(|| EngineError::Isolate("request timeout is too large".into()))?;
    if let Some(len) = request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && len > limits.max_request_bytes as u64
    {
        return Err(HttpError::BodyTooLarge(limits.max_request_bytes));
    }
    let upgrade = websocket_enabled
        && request.version() == hyper::Version::HTTP_11
        && is_websocket_upgrade(&request);
    let ws_key = upgrade
        .then(|| {
            request
                .headers()
                .get(hyper::header::SEC_WEBSOCKET_KEY)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .flatten();
    let method = request.method().as_str().to_owned();
    let uri = request.uri().clone();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned())
        })
        .collect();
    let url = uri.to_string();
    let url = if url.starts_with('/') { format!("http://tysel.local{url}") } else { url };
    let (tx, rx) = mpsc::channel(STREAM_WINDOW);
    let (ws_to_js_tx, ws_to_js_rx, ws_from_js_tx, ws_from_js_rx) = if upgrade {
        let (to_js_tx, to_js_rx) = mpsc::channel(STREAM_WINDOW);
        let (from_js_tx, from_js_rx) = mpsc::channel(STREAM_WINDOW);
        (Some(to_js_tx), Some(to_js_rx), Some(from_js_tx), Some(from_js_rx))
    } else {
        (None, None, None, None)
    };
    let pending_upgrade = if upgrade {
        drop(tx);
        Some(request)
    } else {
        let incoming = request.into_body();
        if incoming.is_end_stream() {
            drop(tx);
        } else {
            tokio::spawn(async move {
                pump_request_body(Limited::new(incoming, limits.max_request_bytes), tx).await;
            });
        }
        None
    };

    let (head, body) = match pool
        .dispatch_incoming(
            IncomingHttp {
                method,
                url,
                headers,
                body: rx,
                ws_in: ws_to_js_rx,
                ws_out: ws_from_js_tx,
                request_id,
            },
            deadline,
        )
        .await
    {
        Ok(pair) => pair,
        Err(EngineError::BodyTooLarge) => {
            return Err(HttpError::BodyTooLarge(limits.max_request_bytes));
        }
        Err(err) => return Err(err.into()),
    };

    if let (Some(request), Some(key), Some(ws_to_js_tx), Some(ws_from_js_rx)) =
        (pending_upgrade, ws_key, ws_to_js_tx, ws_from_js_rx)
        && head.websocket
        && head.status == 101
    {
        let websocket_permit = permit.take();
        let guard = shutdown.as_ref().map(HttpShutdown::track);
        tokio::spawn(async move {
            let _guard = guard;
            let _permit = websocket_permit;
            let pump = async {
                if let Ok(upgraded) = hyper::upgrade::on(request).await {
                    pump_websocket(upgraded, ws_to_js_tx, ws_from_js_rx, shutdown.clone()).await;
                }
            };
            if let Some(shutdown) = &shutdown {
                tokio::select! {
                    () = pump => {}
                    () = shutdown.notified(2) => {}
                }
            } else {
                pump.await;
            }
        });
        let accept = derive_accept_key(key.as_bytes());
        let mut builder = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::SEC_WEBSOCKET_ACCEPT, accept);
        for (name, value) in head.headers {
            builder = builder.header(name, value);
        }
        return builder.body(HttpBody::empty()).map_err(|err| HttpError::Hyper(err.to_string()));
    }

    let mut builder = Response::builder().status(head.status);
    for (name, value) in head.headers {
        builder = builder.header(name, value);
    }
    let body = match body {
        OutgoingHttpBody::Buffered(bytes) => {
            if bytes.len() > limits.max_response_bytes {
                return Err(HttpError::ResponseTooLarge(limits.max_response_bytes));
            }
            HttpBody::once(bytes)
        }
        OutgoingHttpBody::Stream(chunks) => HttpBody::stream(chunks, limits.max_response_bytes),
        OutgoingHttpBody::CheckedStream { chunks, completion } => {
            let mut body = HttpBody::stream(chunks, limits.max_response_bytes);
            body.completion = Some(completion);
            body
        }
    };
    builder.body(body).map_err(|err| HttpError::Hyper(err.to_string()))
}

fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
    if request.method() != hyper::Method::GET {
        return false;
    }
    let upgrade = request
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let connection = request
        .headers()
        .get(hyper::header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("upgrade"));
    let version = request
        .headers()
        .get(hyper::header::SEC_WEBSOCKET_VERSION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim() == "13");
    let key = request.headers().get(hyper::header::SEC_WEBSOCKET_KEY).is_some();
    upgrade && connection && version && key
}

async fn pump_websocket(
    upgraded: hyper::upgrade::Upgraded,
    to_js: mpsc::Sender<Result<Vec<u8>, String>>,
    mut from_js: mpsc::Receiver<Vec<u8>>,
    shutdown: Option<HttpShutdown>,
) {
    let mut ws = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
    let draining = {
        let pump = async {
            loop {
                tokio::select! {
                    incoming = ws.next() => {
                        match incoming {
                            Some(Ok(Message::Text(text))) => {
                                if to_js.send(Ok(text.as_bytes().to_vec())).await.is_err() {
                                    break;
                                }
                            }
                            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                            Some(Ok(Message::Binary(_))) => {}
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        }
                    }
                    outgoing = from_js.recv() => {
                        match outgoing {
                            Some(bytes) => {
                                let text = String::from_utf8_lossy(&bytes).into_owned();
                                if ws.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            None => {
                                let _ = ws.close(None).await;
                                break;
                            }
                        }
                    }
                }
            }
        };
        if let Some(shutdown) = &shutdown {
            tokio::select! {
                () = pump => false,
                () = shutdown.notified(1) => true,
            }
        } else {
            pump.await;
            false
        }
    };
    if draining {
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        let _ = ws
            .close(Some(CloseFrame { code: CloseCode::Away, reason: "server shutdown".into() }))
            .await;
    }
}

async fn pump_request_body(mut body: Limited<Incoming>, tx: mpsc::Sender<Result<Vec<u8>, String>>) {
    loop {
        let frame = tokio::select! {
            () = tx.closed() => return,
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
                let message = if err.downcast_ref::<LengthLimitError>().is_some() {
                    EngineError::BodyTooLarge.to_string()
                } else {
                    err.to_string()
                };
                let _ = tx.send(Err(message)).await;
                return;
            }
            None => return,
        }
    }
}

enum HttpBodyKind {
    Once(Option<Bytes>),
    Stream { rx: mpsc::Receiver<Vec<u8>>, remaining: usize },
}

pub struct HttpBody {
    completion: Option<oneshot::Receiver<Result<(), EngineError>>>,
    kind: HttpBodyKind,
    permit: Option<AdmissionPermit>,
}

impl HttpBody {
    fn once(bytes: Vec<u8>) -> Self {
        Self { completion: None, kind: HttpBodyKind::Once(Some(Bytes::from(bytes))), permit: None }
    }

    fn empty() -> Self {
        Self { completion: None, kind: HttpBodyKind::Once(None), permit: None }
    }

    fn stream(rx: mpsc::Receiver<Vec<u8>>, limit: usize) -> Self {
        Self { completion: None, kind: HttpBodyKind::Stream { rx, remaining: limit }, permit: None }
    }

    fn hold_permit(&mut self, permit: AdmissionPermit) {
        self.permit = Some(permit);
    }
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match &mut this.kind {
            HttpBodyKind::Once(slot) => match slot.take() {
                Some(bytes) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                None => {
                    this.permit.take();
                    Poll::Ready(None)
                }
            },
            HttpBodyKind::Stream { rx, remaining } => match rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) if chunk.len() <= *remaining => {
                    *remaining -= chunk.len();
                    Poll::Ready(Some(Ok(Frame::data(Bytes::from(chunk)))))
                }
                Poll::Ready(Some(_)) => {
                    rx.close();
                    this.permit.take();
                    Poll::Ready(Some(Err(io::Error::other("response body limit exceeded"))))
                }
                Poll::Ready(None) => {
                    if let Some(completion) = &mut this.completion {
                        let result = match Pin::new(completion).poll(cx) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(result) => result,
                        };
                        this.completion = None;
                        this.permit.take();
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                return Poll::Ready(Some(Err(io::Error::other(error.to_string()))));
                            }
                            Err(_) => {
                                return Poll::Ready(Some(Err(io::Error::other(
                                    "response stream interrupted",
                                ))));
                            }
                        }
                    }
                    this.permit.take();
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn size_hint(&self) -> SizeHint {
        match &self.kind {
            HttpBodyKind::Once(Some(bytes)) => SizeHint::with_exact(bytes.len() as u64),
            HttpBodyKind::Once(None) => SizeHint::with_exact(0),
            HttpBodyKind::Stream { .. } => SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;

    #[tokio::test]
    async fn streaming_body_holds_admission_until_end_of_stream() {
        let admission = Arc::new(Admission::new(1));
        let permit = admission.try_acquire().unwrap();
        let (tx, rx) = mpsc::channel(1);
        let mut body = HttpBody::stream(rx, usize::MAX);
        body.hold_permit(permit);
        tx.send(b"chunk".to_vec()).await.unwrap();
        drop(tx);

        assert!(admission.try_acquire().is_none());
        assert!(body.frame().await.is_some());
        assert!(admission.try_acquire().is_none());
        assert!(body.frame().await.is_none());
        assert_eq!(admission.available(), 1);
    }

    #[tokio::test]
    async fn streaming_body_aborts_and_releases_admission_at_response_limit() {
        let admission = Arc::new(Admission::new(1));
        let permit = admission.try_acquire().unwrap();
        let (tx, rx) = mpsc::channel(2);
        let mut body = HttpBody::stream(rx, 4);
        body.hold_permit(permit);
        tx.send(b"abc".to_vec()).await.unwrap();
        tx.send(b"de".to_vec()).await.unwrap();

        assert!(body.frame().await.unwrap().is_ok());
        assert!(body.frame().await.unwrap().is_err());
        assert_eq!(admission.available(), 1);
    }

    #[test]
    fn lowering_limit_accounts_for_existing_permits() {
        let admission = Arc::new(Admission::new(2));
        let first = admission.try_acquire().unwrap();
        let second = admission.try_acquire().unwrap();

        admission.set_limit(1);
        assert_eq!(admission.available(), 0);
        assert!(admission.try_acquire().is_none());
        drop(first);
        assert!(admission.try_acquire().is_none());
        drop(second);
        assert_eq!(admission.available(), 1);
    }
}

#[cfg(test)]
mod stream_completion_tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn stream_error_is_not_reported_as_clean_eof() {
        let (tx, rx) = mpsc::channel(1);
        let (done, completion) = oneshot::channel();
        let mut body = HttpBody::stream(rx, 1024);
        body.completion = Some(completion);
        tx.send(vec![65]).await.unwrap();
        drop(tx);
        done.send(Err(EngineError::Isolate("broken stream".into()))).unwrap();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            Bytes::from_static(b"A")
        );
        assert!(body.frame().await.unwrap().unwrap_err().to_string().contains("broken stream"));
        assert!(body.frame().await.is_none());
    }
}
