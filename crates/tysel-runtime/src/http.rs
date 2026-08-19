use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::{Body, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tysel_engine::{EngineError, HttpRequest};
use tysel_engine_qjs::IsolatePool;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hyper: {0}")]
    Hyper(String),
}

pub async fn serve(listener: TcpListener, pool: Arc<IsolatePool>) -> Result<(), HttpError> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let pool = pool.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let pool = pool.clone();
                async move { Ok::<_, Infallible>(dispatch(pool, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .keep_alive(true)
                .serve_connection(io, service)
                .await;
        });
    }
}

pub async fn bind(addr: SocketAddr, pool: Arc<IsolatePool>) -> Result<SocketAddr, HttpError> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = serve(listener, pool).await;
    });
    Ok(local)
}

async fn dispatch(pool: Arc<IsolatePool>, request: Request<Incoming>) -> Response<HttpBody> {
    match dispatch_inner(pool, request).await {
        Ok(response) => response,
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
) -> Result<Response<HttpBody>, HttpError> {
    let method = request.method().as_str().to_owned();
    let uri = request.uri().clone();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned()))
        .collect();
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|err| HttpError::Hyper(err.to_string()))?
        .to_bytes();
    let url = uri.to_string();
    let url = if url.starts_with('/') {
        format!("http://tysel.local{url}")
    } else {
        url
    };

    let (head, chunks) = pool
        .dispatch(HttpRequest { method, url, headers, body: body.to_vec() })
        .await?;

    let mut builder = Response::builder().status(head.status);
    for (name, value) in head.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(HttpBody::stream(chunks))
        .map_err(|err| HttpError::Hyper(err.to_string()))
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