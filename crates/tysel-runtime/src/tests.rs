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

use crate::http::{SharedPool, bind, bind_with, bind_with_request_limit, handle_stream};

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
async fn keep_alive_uses_replaced_pool() {
    let first = IsolatePool::spawn(
        1,
        r#"export default { async fetch() { return new Response("first"); } };"#,
        config(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let pool = SharedPool::new(Arc::new(first), 16 * 1024 * 1024);
    let accept = pool.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            handle_stream(stream, accept.clone());
        }
    });
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    let (first_status, first_body) = send(&mut sender, "/").await;
    let second_isolate = IsolatePool::spawn(
        1,
        r#"export default { async fetch() { return new Response("second"); } };"#,
        config(),
    )
    .unwrap();
    pool.replace(Arc::new(second_isolate), 16 * 1024 * 1024);
    let (second_status, second_body) = send(&mut sender, "/").await;
    assert_eq!(first_status, 200);
    assert_eq!(second_status, 200);
    assert_eq!(first_body, "first");
    assert_eq!(second_body, "second");
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

const WS_ECHO: &str = r#"
export default {
  async fetch() {
    const socket = tysel.acceptWebSocket();
    socket.addEventListener("message", (event) => {
      socket.send(event.data);
    });
    return new Response(null, { status: 101 });
  },
};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn websocket_echo_roundtrip() {
    let pool = IsolatePool::spawn(1, WS_ECHO, config()).unwrap();
    let addr = bind_with("127.0.0.1:0".parse().unwrap(), Arc::new(pool), 16 * 1024 * 1024, true)
        .await
        .unwrap();
    let url = format!("ws://{addr}/ws");
    let (mut socket, response) = tokio_tungstenite::connect_async(&url).await.expect("connect");
    assert_eq!(response.status(), 101);
    use futures_util::{SinkExt, StreamExt};
    socket.send(tokio_tungstenite::tungstenite::Message::Text("ping".into())).await.unwrap();
    let echoed = socket.next().await.expect("frame").expect("ok");
    assert_eq!(echoed.into_text().expect("text").as_str(), "ping");
    let _ = socket.close(None).await;
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
