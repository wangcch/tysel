use std::collections::HashSet;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use http_body_util::{BodyExt, Empty, Full};
use hyper::Request;
use hyper::body::{Body, Bytes, Frame};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::IsolatePool;

use crate::http::{bind, bind_with_request_limit};

const HANDLER: &str = r#"
export default {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/stream") {
      return new Response(["alpha", "beta", "gamma"]);
    }
    if (path === "/echo") {
      return new Response(await request.text());
    }
    return Response.json({
      message: "Hello from Tysel",
      path,
      isolate: tysel.isolateId,
    });
  },
};
"#;

fn config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 2_000,
        cpu_ms_per_turn: 200,
        memory_limit_bytes: 8 * 1024 * 1024,
    }
}

#[test]
fn crate_is_named() {
    assert!(!super::crate_name().is_empty());
}

#[tokio::test]
async fn fetch_handler_serves_json() {
    let addr = spawn_server(1).await;
    let (status, body) = request(addr, "/hello").await;
    assert_eq!(status, 200);
    assert!(body.contains("\"path\":\"/hello\""));
    assert!(body.contains("Hello from Tysel"));
}

#[tokio::test]
async fn streaming_response_is_concatenated_from_chunks() {
    let addr = spawn_server(1).await;
    let (status, body) = request(addr, "/stream").await;
    assert_eq!(status, 200);
    assert_eq!(body, "alphabetagamma");
}

#[tokio::test]
async fn keep_alive_reuses_http1_connection() {
    let addr = spawn_server(1).await;
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    let (first_status, first) = send(&mut sender, "/one").await;
    let (second_status, second) = send(&mut sender, "/two").await;
    assert_eq!(first_status, 200);
    assert_eq!(second_status, 200);
    assert!(first.contains("/one"));
    assert!(second.contains("/two"));
}

#[tokio::test]
async fn multi_isolate_handles_concurrent_requests() {
    let addr = spawn_server(2).await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        tasks.push(tokio::spawn(async move { request(addr, "/hello").await.1 }));
    }
    let mut isolates = HashSet::new();
    for task in tasks {
        let body = task.await.unwrap();
        if body.contains("\"isolate\":0") {
            isolates.insert(0);
        }
        if body.contains("\"isolate\":1") {
            isolates.insert(1);
        }
    }
    assert!(isolates.len() >= 2, "expected both isolates, got {isolates:?}");
}

#[tokio::test]
async fn oversized_request_body_is_rejected() {
    let pool = IsolatePool::spawn(1, HANDLER, config()).unwrap();
    let addr =
        bind_with_request_limit("127.0.0.1:0".parse().unwrap(), Arc::new(pool), 32).await.unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(
            stream,
        ))
        .await
        .unwrap();
    tokio::spawn(conn);
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header(hyper::header::HOST, "localhost")
        .body(http_body_util::Full::new(Bytes::from(vec![0u8; 64])))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 413);
}

#[tokio::test]
async fn streamed_post_body_reaches_the_handler() {
    let addr = spawn_server(1).await;
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(hyper::header::HOST, "localhost")
        .body(Full::new(Bytes::from_static(b"hello")))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"hello");
}

#[tokio::test]
async fn chunked_oversized_body_is_rejected() {
    let pool = IsolatePool::spawn(1, HANDLER, config()).unwrap();
    let addr =
        bind_with_request_limit("127.0.0.1:0".parse().unwrap(), Arc::new(pool), 32).await.unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, ChunkList>(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(hyper::header::HOST, "localhost")
        .header(hyper::header::TRANSFER_ENCODING, "chunked")
        .body(ChunkList { parts: vec![Bytes::from(vec![b'x'; 16]); 4].into_iter() })
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 413);
}

struct ChunkList {
    parts: std::vec::IntoIter<Bytes>,
}

impl Body for ChunkList {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut().parts.next() {
            Some(data) => Poll::Ready(Some(Ok(Frame::data(data)))),
            None => Poll::Ready(None),
        }
    }
}

async fn spawn_server(workers: usize) -> std::net::SocketAddr {
    let pool = IsolatePool::spawn(workers, HANDLER, config()).unwrap();
    bind("127.0.0.1:0".parse().unwrap(), Arc::new(pool)).await.unwrap()
}

async fn request(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    send(&mut sender, path).await
}

async fn send(
    sender: &mut hyper::client::conn::http1::SendRequest<Empty<Bytes>>,
    path: &str,
) -> (u16, String) {
    let request = Request::builder()
        .uri(path)
        .header(hyper::header::HOST, "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
