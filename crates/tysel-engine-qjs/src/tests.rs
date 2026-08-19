use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use hyper::body::Frame;
use hyper::{Request as HyperRequest, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tysel_engine::{EngineError, HttpRequest, InterruptReason, IsolateConfig, Value};

use crate::{IncomingHttp, IsolateCancel, IsolatePool, STREAM_WINDOW, eval, eval_cancellable};

fn config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 2_000,
        cpu_ms_per_turn: 50,
        memory_limit_bytes: 8 * 1024 * 1024,
    }
}

#[test]
fn promise_resolves_from_rust_async_echo() {
    let value = eval(
        r#"
        (async () => {
            const first = await tysel.echo("hello");
            const second = await tysel.sleep(10);
            return first;
        })()
        "#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::String("hello".into()));
}

#[test]
fn secret_ref_returns_opaque_handle() {
    let value = eval(r#"(async () => tysel.secretRef("db"))()"#, config()).expect("eval");
    assert_eq!(value, Value::String("secret:db".into()));
}

#[test]
fn text_encoder_roundtrips_utf8() {
    let value = eval(
        r#"(() => {
            const bytes = new TextEncoder().encode("你好");
            return new TextDecoder().decode(bytes);
        })()"#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::String("你好".into()));
}

#[test]
fn text_decoder_rejects_non_utf8() {
    let value = eval(
        r#"(() => {
            try {
                new TextDecoder("latin1");
                return "accepted";
            } catch (err) {
                return String(err);
            }
        })()"#,
        config(),
    )
    .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("utf-8"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn crypto_get_random_values_fills_buffer() {
    let value = eval(
        r#"(() => {
            const first = crypto.getRandomValues(new Uint8Array(16));
            const second = crypto.getRandomValues(new Uint8Array(16));
            return Array.from(first).join(",") !== Array.from(second).join(",");
        })()"#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn crypto_get_random_values_enforces_quota() {
    let value = eval(
        r#"(() => {
            try {
                crypto.getRandomValues(new Uint8Array(65537));
                return "accepted";
            } catch (err) {
                return String(err);
            }
        })()"#,
        config(),
    )
    .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("QuotaExceededError"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn set_timeout_resolves_after_delay() {
    let value = eval(
        r#"(async () => {
            const started = Date.now();
            const result = await new Promise((resolve) => setTimeout(resolve, 20, "ok"));
            if (result !== "ok") return "bad";
            return Date.now() - started >= 15 ? "ok" : "early";
        })()"#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::String("ok".into()));
}

#[test]
fn clear_timeout_skips_callback() {
    let value = eval(
        r#"(async () => {
            let fired = 0;
            const id = setTimeout(() => { fired = 1; }, 30);
            clearTimeout(id);
            await tysel.sleep(50);
            return fired;
        })()"#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::Number(0.0));
}

#[test]
fn set_interval_can_be_cleared() {
    let value = eval(
        r#"(async () => {
            let n = 0;
            await new Promise((resolve) => {
                const id = setInterval(() => {
                    n += 1;
                    if (n >= 2) {
                        clearInterval(id);
                        resolve();
                    }
                }, 15);
            });
            return n;
        })()"#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::Number(2.0));
}

#[test]
fn await_set_timeout_does_not_consume_cpu_budget() {
    let value = eval(
        r#"(async () => {
            await new Promise((resolve) => setTimeout(resolve, 80));
            return "ok";
        })()"#,
        IsolateConfig { cpu_ms_per_turn: 20, request_timeout_ms: 2_000, ..config() },
    )
    .expect("I/O wait should not exhaust the JS CPU budget");
    assert_eq!(value, Value::String("ok".into()));
}

#[test]
fn cancel_stops_pending_io() {
    let cancel = IsolateCancel::new();
    let cancel_for_eval = cancel.clone();
    let started = Instant::now();
    let handle = thread::spawn(move || {
        eval_cancellable("(async () => tysel.sleep(5000))()", config(), cancel_for_eval)
    });
    thread::sleep(Duration::from_millis(30));
    cancel.cancel();
    let err = handle.join().expect("join").expect_err("cancelled");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(err, EngineError::Interrupted(InterruptReason::Cancelled)));
}

#[test]
fn request_timeout_interrupts_sleep() {
    let err = eval(
        "(async () => tysel.sleep(5000))()",
        IsolateConfig { request_timeout_ms: 40, ..config() },
    )
    .expect_err("timeout");
    assert!(matches!(err, EngineError::Interrupted(InterruptReason::Timeout)));
}

#[test]
fn await_does_not_consume_cpu_budget() {
    let value = eval(
        r#"(async () => { await tysel.sleep(80); return "ok"; })()"#,
        IsolateConfig { cpu_ms_per_turn: 20, request_timeout_ms: 2_000, ..config() },
    )
    .expect("I/O wait should not exhaust the JS CPU budget");
    assert_eq!(value, Value::String("ok".into()));
}

#[test]
fn cpu_interrupt_stops_busy_loop() {
    let started = Instant::now();
    let err = eval(
        "(() => { let x = 0; for (;;) { x++; } })()",
        IsolateConfig { cpu_ms_per_turn: 15, request_timeout_ms: 1_000, ..config() },
    )
    .expect_err("cpu interrupt");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(
        err,
        EngineError::Interrupted(InterruptReason::Timeout | InterruptReason::Cancelled)
    ));
}

#[test]
fn memory_limit_rejects_large_allocation() {
    let err = eval(
        "(() => { const chunks = []; for (let i = 0; i < 64; i++) { chunks.push(new Uint8Array(1024 * 1024)); } return chunks.length; })()",
        IsolateConfig { memory_limit_bytes: 2 * 1024 * 1024, ..config() },
    )
    .expect_err("memory limit");
    match err {
        EngineError::Interrupted(InterruptReason::MemoryLimit) | EngineError::Isolate(_) => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

const FETCH_HANDLER: &str = r#"
export default {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/stream") {
      return new Response(["alpha", "beta", "gamma"]);
    }
    return Response.json({
      message: "Hello from Tysel",
      path,
      isolate: tysel.isolateId,
    });
  },
};
"#;

#[tokio::test]
async fn fetch_handler_streams_body_chunks() {
    let pool = IsolatePool::spawn(1, FETCH_HANDLER, config()).expect("spawn isolate");
    let (head, mut body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/stream".into(),
            headers: vec![],
            body: vec![],
        })
        .await
        .expect("dispatch");
    assert_eq!(head.status, 200);
    let mut chunks = Vec::new();
    while let Some(chunk) = body.recv().await {
        chunks.push(String::from_utf8(chunk).expect("utf8 chunk"));
    }
    assert_eq!(chunks, ["alpha", "beta", "gamma"]);
}

const SLEEP_HANDLER: &str = r#"
export default {
  async fetch() {
    await tysel.sleep(80);
    return new Response("slept");
  },
};
"#;

#[tokio::test]
async fn fetch_handler_sleep_does_not_exhaust_cpu_budget() {
    let pool = IsolatePool::spawn(
        1,
        SLEEP_HANDLER,
        IsolateConfig { cpu_ms_per_turn: 20, request_timeout_ms: 2_000, ..config() },
    )
    .expect("spawn isolate");
    let (head, mut body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: vec![],
        })
        .await
        .expect("dispatch");
    assert_eq!(head.status, 200);
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    assert_eq!(String::from_utf8(bytes).expect("utf8"), "slept");
}

const HEADERS_HANDLER: &str = r#"
export default {
  fetch() {
    const headers = new Headers([
      ["X-Name", "tysel"],
      ["Content-Type", "text/plain"],
    ]);
    return new Response(headers.get("x-name"), {
      headers: [
        ["content-type", "text/plain"],
        ["x-echo", headers.get("content-type")],
      ],
    });
  },
};
"#;

#[tokio::test]
async fn headers_accepts_sequence_initializer() {
    let pool = IsolatePool::spawn(1, HEADERS_HANDLER, config()).expect("spawn isolate");
    let (head, mut body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: vec![],
        })
        .await
        .expect("dispatch");
    assert_eq!(head.status, 200);
    let content_type = head
        .headers
        .iter()
        .find(|(name, _)| name == "content-type")
        .map(|(_, value)| value.as_str());
    assert_eq!(content_type, Some("text/plain"));
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    assert_eq!(String::from_utf8(bytes).expect("utf8"), "tysel");
}

const ECHO_BODY: &str = r#"
export default {
  async fetch(request) {
    return new Response(await request.text());
  },
};
"#;

#[tokio::test]
async fn fetch_handler_reads_streamed_request_body() {
    let pool = IsolatePool::spawn(1, ECHO_BODY, config()).expect("spawn isolate");
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let dispatch = pool.dispatch_incoming(IncomingHttp {
        method: "POST".into(),
        url: "http://tysel.local/".into(),
        headers: vec![],
        body: rx,
        ws_in: None,
        ws_out: None,
    });
    tx.send(Ok(b"hel".to_vec())).await.unwrap();
    tx.send(Ok(b"lo".to_vec())).await.unwrap();
    drop(tx);
    let (head, mut body) = dispatch.await.expect("dispatch");
    assert_eq!(head.status, 200);
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    assert_eq!(String::from_utf8(bytes).expect("utf8"), "hello");
}

const DELAY_ECHO: &str = r#"
export default {
  async fetch(request) {
    await tysel.sleep(80);
    return new Response(await request.text());
  },
};
"#;

#[tokio::test]
async fn streamed_request_body_applies_backpressure() {
    let pool = IsolatePool::spawn(1, DELAY_ECHO, config()).expect("spawn isolate");
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let dispatch = tokio::spawn(async move {
        pool.dispatch_incoming(IncomingHttp {
            method: "POST".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: rx,
            ws_in: None,
            ws_out: None,
        })
        .await
    });
    let started = Instant::now();
    for _ in 0..(STREAM_WINDOW + 4) {
        tx.send(Ok(vec![b'a'])).await.unwrap();
    }
    drop(tx);
    assert!(
        started.elapsed() >= Duration::from_millis(40),
        "producer finished too quickly: {:?}",
        started.elapsed()
    );
    let (head, mut body) = dispatch.await.expect("join").expect("dispatch");
    assert_eq!(head.status, 200);
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    assert_eq!(bytes.len(), STREAM_WINDOW + 4);
}

#[tokio::test]
async fn oversized_streamed_body_is_body_too_large() {
    let pool = IsolatePool::spawn(1, ECHO_BODY, config()).expect("spawn isolate");
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let dispatch = pool.dispatch_incoming(IncomingHttp {
        method: "POST".into(),
        url: "http://tysel.local/".into(),
        headers: vec![],
        body: rx,
        ws_in: None,
        ws_out: None,
    });
    tx.send(Ok(b"ok".to_vec())).await.unwrap();
    tx.send(Err(EngineError::BodyTooLarge.to_string())).await.unwrap();
    drop(tx);
    let err = dispatch.await.expect_err("limit");
    assert!(matches!(err, EngineError::BodyTooLarge), "error was {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn outbound_http_get_reads_body() {
    let addr = serve_bytes(Bytes::from_static(b"hello"));
    let url = format!("http://{addr}/");
    let value = tokio::task::spawn_blocking(move || {
        eval(&format!("(async () => (await tysel.httpGet(\"{url}\")).text())()"), config())
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("hello".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_outbound_fetch() {
    let addr = serve_slow();
    let url = format!("http://{addr}/");
    let cancel = IsolateCancel::new();
    let cancel_for_eval = cancel.clone();
    let started = Instant::now();
    let handle = tokio::task::spawn_blocking(move || {
        eval_cancellable(
            &format!("(async () => (await tysel.httpGet(\"{url}\")).text())()"),
            config(),
            cancel_for_eval,
        )
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    cancel.cancel();
    let err = handle.await.expect("join").expect_err("cancelled");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        matches!(err, EngineError::Interrupted(InterruptReason::Cancelled))
            || err.to_string().contains("Cancelled"),
        "error was {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn outbound_fetch_body_applies_backpressure() {
    let polled = std::sync::Arc::new(AtomicUsize::new(0));
    // More payload than a typical localhost TCP window so Linux cannot hide
    // a missing mpsc bound behind kernel buffering (32 × 128KiB ≈ 4MiB).
    let chunks = STREAM_WINDOW * 16;
    let addr = serve_counted(chunks, polled.clone());
    let url = format!("http://{addr}/");
    let eval = tokio::task::spawn_blocking(move || {
        eval(
            &format!(
                r#"(async () => {{
                    const res = await tysel.httpGet("{url}");
                    await tysel.sleep(80);
                    let n = 0;
                    for (;;) {{
                        const chunk = await tysel._httpRead();
                        if (chunk == null) break;
                        n += chunk.length;
                    }}
                    return n;
                }})()"#
            ),
            config(),
        )
    });
    let started = Instant::now();
    while polled.load(AtomicOrdering::SeqCst) == 0 && started.elapsed() < Duration::from_secs(1) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let during = polled.load(AtomicOrdering::SeqCst);
    assert!(during > 0, "origin never polled");
    assert!(during < chunks, "producer ran ahead: {during}");
    let value = eval.await.expect("join").expect("eval");
    assert_eq!(value, Value::Number((chunks * COUNTED_CHUNK_LEN) as f64));
    assert_eq!(polled.load(AtomicOrdering::SeqCst), chunks);
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_follows_http_redirect() {
    let addr = serve_redirect();
    let url = format!("http://{addr}/go");
    let value = tokio::task::spawn_blocking(move || {
        eval(&format!("(async () => (await fetch(\"{url}\")).text())()"), config())
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("hello".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_https_is_not_rejected_as_unsupported() {
    let err = tokio::task::spawn_blocking(|| {
        eval("(async () => (await fetch(\"https://127.0.0.1:1/\")).text())()", config())
    })
    .await
    .expect("join")
    .expect_err("connect");
    let message = err.to_string();
    assert!(
        !message.contains("only supports http") || message.contains("https"),
        "https was rejected as unsupported: {message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_rejects_post() {
    let value = tokio::task::spawn_blocking(|| {
        eval(
            r#"(async () => {
                try {
                    await fetch("http://127.0.0.1:1/", { method: "POST" });
                    return "accepted";
                } catch (err) {
                    return String(err);
                }
            })()"#,
            config(),
        )
    })
    .await
    .expect("join")
    .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("GET and HEAD"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
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
async fn accepted_websocket_echoes_text() {
    let pool = IsolatePool::spawn(1, WS_ECHO, config()).expect("spawn isolate");
    let (to_js_tx, to_js_rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let (from_js_tx, mut from_js_rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(tx);
    let dispatch = pool.dispatch_incoming(IncomingHttp {
        method: "GET".into(),
        url: "http://tysel.local/ws".into(),
        headers: vec![],
        body: rx,
        ws_in: Some(to_js_rx),
        ws_out: Some(from_js_tx),
    });
    let (head, _body) = dispatch.await.expect("dispatch");
    assert_eq!(head.status, 101);
    assert!(head.websocket);
    to_js_tx.send(Ok(b"ping".to_vec())).await.unwrap();
    let echoed = from_js_rx.recv().await.expect("echo");
    assert_eq!(echoed, b"ping");
    drop(to_js_tx);
}

fn serve_bytes(body: Bytes) -> SocketAddr {
    spawn_origin(move |_| {
        let body = body.clone();
        async move { Ok::<_, Infallible>(Response::new(http_body_util::Full::new(body))) }
    })
}

fn serve_slow() -> SocketAddr {
    spawn_origin(|_| async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok::<_, Infallible>(Response::new(http_body_util::Full::new(Bytes::from_static(b"late"))))
    })
}

fn serve_redirect() -> SocketAddr {
    spawn_origin(|req| {
        let path = req.uri().path().to_owned();
        async move {
            if path == "/go" {
                Ok(Response::builder()
                    .status(302)
                    .header("location", "/done")
                    .body(http_body_util::Full::new(Bytes::new()))
                    .unwrap())
            } else {
                Ok(Response::new(http_body_util::Full::new(Bytes::from_static(b"hello"))))
            }
        }
    })
}

fn serve_counted(chunks: usize, polled: std::sync::Arc<AtomicUsize>) -> SocketAddr {
    spawn_origin(move |_| {
        let polled = polled.clone();
        async move {
            Ok::<_, Infallible>(Response::new(CountedBody {
                left: chunks,
                polled,
                yield_next: false,
            }))
        }
    })
}

fn spawn_origin<F, Fut, B>(handler: F) -> SocketAddr
where
    F: Fn(HyperRequest<hyper::body::Incoming>) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<Response<B>, Infallible>> + Send + 'static,
    B: hyper::body::Body<Data = Bytes, Error = Infallible> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("origin runtime")
            .block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind origin");
                tx.send(listener.local_addr().expect("local addr")).expect("addr");
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        let service = hyper::service::service_fn(handler);
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            });
    });
    rx.recv().expect("origin addr")
}

const COUNTED_CHUNK_LEN: usize = 128 * 1024;

struct CountedBody {
    left: usize,
    polled: std::sync::Arc<AtomicUsize>,
    yield_next: bool,
}

impl hyper::body::Body for CountedBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.yield_next {
            this.yield_next = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        if this.left == 0 {
            return Poll::Ready(None);
        }
        this.left -= 1;
        this.polled.fetch_add(1, AtomicOrdering::SeqCst);
        this.yield_next = true;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(vec![b'x'; COUNTED_CHUNK_LEN])))))
    }
}
