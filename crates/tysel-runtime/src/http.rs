use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use hyper::body::{Body, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tysel_engine::{EngineError, HttpHead, HttpRequest, IsolateConfig};
use tysel_engine_qjs::{IncomingHttp, IsolatePool, STREAM_WINDOW};
use tysel_isolate::{IsolatedHttpPool, MAX_ISOLATED_HTTP_BODY, locate_worker};
use tysel_package::default_max_request_bytes;

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
    ) -> Result<(tysel_engine::HttpHead, mpsc::Receiver<Vec<u8>>), EngineError> {
        match self {
            Self::Trusted(pool) => pool.dispatch_incoming(request).await,
            Self::Isolated(pool) => dispatch_isolated(pool.clone(), request).await,
        }
    }
}

pub fn spawn_app_isolate(
    execution_profile: &str,
    source: &str,
    config: IsolateConfig,
    secret_names: Vec<String>,
) -> Result<AppIsolate, EngineError> {
    if execution_profile.eq_ignore_ascii_case("isolated") {
        let worker = locate_worker().map_err(|err| EngineError::Isolate(err.to_string()))?;
        let pool = IsolatedHttpPool::spawn_from_config(worker, source, config, secret_names)
            .map_err(|err| EngineError::Isolate(err.to_string()))?;
        Ok(AppIsolate::Isolated(Arc::new(pool)))
    } else {
        Ok(AppIsolate::Trusted(Arc::new(IsolatePool::spawn(1, source, config)?)))
    }
}

async fn dispatch_isolated(
    pool: Arc<IsolatedHttpPool>,
    request: IncomingHttp,
) -> Result<(HttpHead, mpsc::Receiver<Vec<u8>>), EngineError> {
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
    let result = tokio::task::spawn_blocking(move || {
        pool.dispatch_sync(HttpRequest {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body,
        })
    })
    .await
    .map_err(|err| EngineError::Isolate(err.to_string()))?;
    let (head, bytes) = result.map_err(|err| EngineError::Isolate(err.to_string()))?;
    let (tx, rx) = mpsc::channel(1);
    if !bytes.is_empty() {
        let _ = tx.try_send(bytes);
    }
    Ok((head, rx))
}

#[derive(Clone)]
pub struct SharedPool {
    inner: Arc<RwLock<(AppIsolate, usize, bool)>>,
}

impl SharedPool {
    pub fn new(pool: impl Into<AppIsolate>, max_request_bytes: usize) -> Self {
        Self::with_websocket(pool, max_request_bytes, false)
    }

    pub fn with_websocket(
        pool: impl Into<AppIsolate>,
        max_request_bytes: usize,
        websocket: bool,
    ) -> Self {
        Self { inner: Arc::new(RwLock::new((pool.into(), max_request_bytes, websocket))) }
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
        let previous = {
            let mut guard = self.inner.write().expect("pool lock");
            std::mem::replace(&mut *guard, (pool.into(), max_request_bytes, websocket))
        };
        drop(previous);
    }

    pub fn current(&self) -> (AppIsolate, usize) {
        let guard = self.inner.read().expect("pool lock");
        (guard.0.clone(), guard.1)
    }

    pub fn websocket(&self) -> bool {
        self.inner.read().expect("pool lock").2
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
    let pool = SharedPool::with_websocket(pool, max_request_bytes, websocket);
    loop {
        let (stream, _peer) = listener.accept().await?;
        handle_stream(stream, pool.clone());
    }
}

pub fn handle_stream(stream: tokio::net::TcpStream, pool: SharedPool) {
    tokio::spawn(async move {
        let io = TokioIo::new(stream);
        let service = service_fn(move |request| {
            let pool = pool.clone();
            async move {
                let method = request.method().as_str().to_owned();
                let path = request.uri().path().to_owned();
                let started = Instant::now();
                let (isolate, max_request_bytes) = pool.current();
                let response =
                    dispatch(isolate, request, max_request_bytes, pool.websocket()).await;
                tysel_observability::log_http(
                    &method,
                    &path,
                    response.status().as_u16(),
                    started.elapsed(),
                );
                Ok::<_, Infallible>(response)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .keep_alive(true)
            .serve_connection(io, service)
            .with_upgrades()
            .await;
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
    max_request_bytes: usize,
    websocket: bool,
) -> Response<HttpBody> {
    match dispatch_inner(pool, request, max_request_bytes, websocket).await {
        Ok(response) => response,
        Err(HttpError::BodyTooLarge(limit)) => Response::builder()
            .status(StatusCode::PAYLOAD_TOO_LARGE)
            .header(hyper::header::CONTENT_TYPE, "text/plain")
            .body(HttpBody::once(format!("request body exceeds {limit} bytes").into_bytes()))
            .unwrap_or_else(|_| Response::new(HttpBody::once(b"payload too large".to_vec()))),
        Err(err) => {
            let body = format!("{err}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(hyper::header::CONTENT_TYPE, "text/plain")
                .body(HttpBody::once(body.into_bytes()))
                .unwrap_or_else(|_| Response::new(HttpBody::once(b"internal error".to_vec())))
        }
    }
}

async fn dispatch_inner(
    pool: AppIsolate,
    request: Request<Incoming>,
    max_request_bytes: usize,
    websocket_enabled: bool,
) -> Result<Response<HttpBody>, HttpError> {
    if let Some(len) = request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if len > max_request_bytes as u64 {
            return Err(HttpError::BodyTooLarge(max_request_bytes));
        }
    }
    let upgrade = websocket_enabled && is_websocket_upgrade(&request);
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
    let (ws_to_js_tx, ws_to_js_rx) = mpsc::channel(STREAM_WINDOW);
    let (ws_from_js_tx, ws_from_js_rx) = mpsc::channel(STREAM_WINDOW);
    let pending_upgrade = if upgrade {
        drop(tx);
        Some(request)
    } else {
        let incoming = request.into_body();
        tokio::spawn(async move {
            pump_request_body(Limited::new(incoming, max_request_bytes), tx).await;
        });
        None
    };

    let (head, chunks) = match pool
        .dispatch_incoming(IncomingHttp {
            method,
            url,
            headers,
            body: rx,
            ws_in: upgrade.then_some(ws_to_js_rx),
            ws_out: upgrade.then_some(ws_from_js_tx),
        })
        .await
    {
        Ok(pair) => pair,
        Err(EngineError::BodyTooLarge) => return Err(HttpError::BodyTooLarge(max_request_bytes)),
        Err(err) => return Err(err.into()),
    };

    if let (Some(request), Some(key)) = (pending_upgrade, ws_key) {
        if head.websocket && head.status == 101 {
            tokio::spawn(async move {
                if let Ok(upgraded) = hyper::upgrade::on(request).await {
                    pump_websocket(upgraded, ws_to_js_tx, ws_from_js_rx).await;
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
            return builder
                .body(HttpBody::Once(None))
                .map_err(|err| HttpError::Hyper(err.to_string()));
        }
    }

    let mut builder = Response::builder().status(head.status);
    for (name, value) in head.headers {
        builder = builder.header(name, value);
    }
    builder.body(HttpBody::stream(chunks)).map_err(|err| HttpError::Hyper(err.to_string()))
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
) {
    let mut ws = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
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
}

async fn pump_request_body(mut body: Limited<Incoming>, tx: mpsc::Sender<Result<Vec<u8>, String>>) {
    loop {
        match body.frame().await {
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

pub enum HttpBody {
    Once(Option<Bytes>),
    Stream(mpsc::Receiver<Vec<u8>>),
}

impl HttpBody {
    fn once(bytes: Vec<u8>) -> Self {
        Self::Once(Some(Bytes::from(bytes)))
    }

    fn stream(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self::Stream(rx)
    }
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            HttpBody::Once(slot) => Poll::Ready(slot.take().map(|bytes| Ok(Frame::data(bytes)))),
            HttpBody::Stream(rx) => match rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => Poll::Ready(Some(Ok(Frame::data(Bytes::from(chunk))))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
