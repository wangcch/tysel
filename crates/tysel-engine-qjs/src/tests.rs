use std::thread;
use std::time::{Duration, Instant};

use tysel_engine::{EngineError, HttpRequest, InterruptReason, IsolateConfig, Value};

use crate::{IsolateCancel, IsolatePool, eval, eval_cancellable};

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
