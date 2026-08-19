use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use hyper::body::{Body, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tysel_engine::EngineError;
use tysel_engine_qjs::{IncomingHttp, IsolatePool, STREAM_WINDOW};
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
pub struct SharedPool {
    inner: Arc<RwLock<(Arc<IsolatePool>, usize)>>,
}

impl SharedPool {
    pub fn new(pool: Arc<IsolatePool>, max_request_bytes: usize) -> Self {
        Self { inner: Arc::new(RwLock::new((pool, max_request_bytes))) }
    }

    pub fn replace(&self, pool: Arc<IsolatePool>, max_request_bytes: usize) {
        let previous = {
            let mut guard = self.inner.write().expect("pool lock");
            std::mem::replace(&mut *guard, (pool, max_request_bytes))
        };
        drop(previous);
    }

    pub fn current(&self) -> (Arc<IsolatePool>, usize) {
        let guard = self.inner.read().expect("pool lock");
        (guard.0.clone(), guard.1)
    }
}

pub async fn serve(
    listener: TcpListener,
    pool: Arc<IsolatePool>,
    max_request_bytes: usize,
) -> Result<(), HttpError> {
    let pool = SharedPool::new(pool, max_request_bytes);
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
                let (isolate, max_request_bytes) = pool.current();
                Ok::<_, Infallible>(dispatch(isolate, request, max_request_bytes).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .keep_alive(true)
            .serve_connection(io, service)
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
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = serve(listener, pool, max_request_bytes).await;
    });
    Ok(local)
}

async fn dispatch(
    pool: Arc<IsolatePool>,
    request: Request<Incoming>,
    max_request_bytes: usize,
) -> Response<HttpBody> {
    match dispatch_inner(pool, request, max_request_bytes).await {
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
    pool: Arc<IsolatePool>,
    request: Request<Incoming>,
    max_request_bytes: usize,
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
    let incoming = request.into_body();
    tokio::spawn(async move {
        pump_request_body(Limited::new(incoming, max_request_bytes), tx).await;
    });

    let (head, chunks) = match pool
        .dispatch_incoming(IncomingHttp { method, url, headers, body: rx })
        .await
    {
        Ok(pair) => pair,
        Err(EngineError::BodyTooLarge) => return Err(HttpError::BodyTooLarge(max_request_bytes)),
        Err(err) => return Err(err.into()),
    };

    let mut builder = Response::builder().status(head.status);
    for (name, value) in head.headers {
        builder = builder.header(name, value);
    }
    builder.body(HttpBody::stream(chunks)).map_err(|err| HttpError::Hyper(err.to_string()))
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
