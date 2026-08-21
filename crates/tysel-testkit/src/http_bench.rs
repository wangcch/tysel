use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, ensure};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpStream;
use tysel_engine::IsolateConfig;
use tysel_engine_qjs::IsolatePool;
use tysel_runtime::serve_with_protocols;

use crate::report::{BenchScale, MetricReport, SuiteReport, metric, suite_report};

const HANDLER: &str = r#"
export default {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/json/1k") {
      return Response.json({ payload: "a".repeat(1024) });
    }
    if (path === "/json/64k") {
      return Response.json({ payload: "b".repeat(65536) });
    }
    if (path === "/bytes/64k") {
      return new Response("x".repeat(65536), {
        headers: { "content-type": "application/octet-stream" },
      });
    }
    if (path === "/stream") {
      return new Response(["alpha", "beta", "gamma"]);
    }
    if (path === "/sse") {
      return new Response(["data: one\n\n", "data: two\n\n", "data: three\n\n"], {
        headers: { "content-type": "text/event-stream" },
      });
    }
    if (path === "/ws") {
      const socket = tysel.acceptWebSocket();
      socket.addEventListener("message", (event) => {
        socket.send(event.data);
      });
      return new Response(null, { status: 101 });
    }
    return Response.json({ ok: true, path });
  },
};
"#;

pub fn run_http(scale: BenchScale) -> Result<SuiteReport> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime for HTTP bench")?;
    runtime.block_on(run_http_async(scale))
}

async fn run_http_async(scale: BenchScale) -> Result<SuiteReport> {
    let addr = spawn_server().await?;
    let mut metrics = Vec::new();
    metrics.push(http1_keepalive(addr, scale).await?);
    metrics.push(http2_json(addr, "/json/1k", "http2_ms", scale).await?);
    metrics.push(http1_path(addr, "/json/1k", "json_1kb_ms", scale).await?);
    metrics.push(http1_path(addr, "/json/64k", "json_64kb_ms", scale).await?);
    metrics.push(http1_path(addr, "/bytes/64k", "bytes_64kb_ms", scale).await?);
    metrics.push(http1_path(addr, "/stream", "streaming_ms", scale).await?);
    metrics.push(websocket_echo(addr, scale).await?);
    metrics.push(http1_path(addr, "/sse", "sse_ms", scale).await?);
    for &concurrency in scale.http1_concurrency {
        metrics.push(concurrency_metric(addr, "http1", concurrency, scale).await?);
    }
    for &concurrency in scale.http2_concurrency {
        metrics.push(concurrency_metric(addr, "http2", concurrency, scale).await?);
    }
    Ok(suite_report("http", metrics))
}

async fn spawn_server() -> Result<SocketAddr> {
    let isolate = IsolatePool::spawn(8, HANDLER, config()).context("spawn HTTP isolate pool")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ =
            serve_with_protocols(listener, Arc::new(isolate), 16 * 1024 * 1024, true, true, true)
                .await;
    });
    // One probe so the listener is accepting before the first sample.
    let (status, _) = http1_once(addr, "/").await?;
    ensure!(status == 200, "HTTP bench server failed probe: {status}");
    Ok(addr)
}

async fn http1_keepalive(addr: SocketAddr, scale: BenchScale) -> Result<MetricReport> {
    let stream = TcpStream::connect(addr).await?;
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream)).await?;
    tokio::spawn(conn);
    let mut samples = Vec::with_capacity(scale.samples);
    for index in 0..scale.samples {
        let started = Instant::now();
        let (status, body) = send_http1(&mut sender, &format!("/ka/{index}")).await?;
        ensure!(status == 200 && body.contains("/ka/"), "keep-alive {index}: {status} {body}");
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(metric("http1_keepalive_ms", "ms", samples))
}

async fn http1_path(
    addr: SocketAddr,
    path: &str,
    name: &str,
    scale: BenchScale,
) -> Result<MetricReport> {
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        let (status, body) = http1_once(addr, path).await?;
        ensure!(status == 200, "{name} status {status}");
        if path == "/stream" {
            ensure!(body == "alphabetagamma", "stream body {body}");
        }
        if path == "/bytes/64k" {
            ensure!(body.len() == 65_536, "bytes body length {}", body.len());
        }
        if path == "/sse" {
            ensure!(body.contains("data: one") && body.contains("data: three"), "sse body {body}");
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(metric(name, "ms", samples))
}

async fn http2_json(
    addr: SocketAddr,
    path: &str,
    name: &str,
    scale: BenchScale,
) -> Result<MetricReport> {
    let stream = TcpStream::connect(addr).await?;
    let (mut sender, conn) = hyper::client::conn::http2::handshake::<_, _, Empty<Bytes>>(
        TokioExecutor::new(),
        TokioIo::new(stream),
    )
    .await?;
    tokio::spawn(conn);
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        let request =
            Request::builder().uri(format!("http://localhost{path}")).body(Empty::new())?;
        let response = sender.send_request(request).await?;
        ensure!(response.version() == hyper::Version::HTTP_2);
        ensure!(response.status().as_u16() == 200);
        let _ = response.into_body().collect().await?.to_bytes();
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(metric(name, "ms", samples))
}

async fn websocket_echo(addr: SocketAddr, scale: BenchScale) -> Result<MetricReport> {
    let url = format!("ws://{addr}/ws");
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        let (mut socket, response) = tokio_tungstenite::connect_async(&url).await?;
        ensure!(response.status() == 101);
        socket.send(tokio_tungstenite::tungstenite::Message::Text("ping".into())).await?;
        let echoed = socket.next().await.context("websocket echo")??;
        ensure!(echoed.into_text()?.as_str() == "ping");
        let _ = socket.close(None).await;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(metric("websocket_echo_ms", "ms", samples))
}

async fn concurrency_metric(
    addr: SocketAddr,
    protocol: &str,
    concurrency: usize,
    scale: BenchScale,
) -> Result<MetricReport> {
    let mut samples = Vec::with_capacity(scale.samples);
    for _ in 0..scale.samples {
        let started = Instant::now();
        match protocol {
            "http1" => http1_concurrent(addr, concurrency).await?,
            "http2" => http2_concurrent(addr, concurrency).await?,
            other => anyhow::bail!("unsupported benchmark protocol {other}"),
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut metric = metric(format!("{protocol}_concurrency_{concurrency}_ms"), "ms", samples);
    metric.extra = Some(serde_json::json!({
        "in_flight": concurrency,
        "protocol": protocol,
    }));
    Ok(metric)
}

async fn http1_concurrent(addr: SocketAddr, n: usize) -> Result<()> {
    let mut joins = Vec::with_capacity(n);
    for _ in 0..n {
        joins.push(tokio::spawn(async move { http1_once(addr, "/").await }));
    }
    for join in joins {
        let (status, _) = join.await??;
        ensure!(status == 200);
    }
    Ok(())
}

async fn http2_concurrent(addr: SocketAddr, n: usize) -> Result<()> {
    const STREAMS_PER_CONNECTION: usize = 32;
    let connection_count = n.div_ceil(STREAMS_PER_CONNECTION);
    let mut senders = Vec::with_capacity(connection_count);
    for _ in 0..connection_count {
        let stream = TcpStream::connect(addr).await?;
        let (sender, conn) = hyper::client::conn::http2::handshake::<_, _, Empty<Bytes>>(
            TokioExecutor::new(),
            TokioIo::new(stream),
        )
        .await?;
        tokio::spawn(conn);
        senders.push(sender);
    }
    let mut joins = Vec::with_capacity(n);
    for index in 0..n {
        let mut sender = senders[index % senders.len()].clone();
        joins.push(tokio::spawn(async move {
            let request = Request::builder().uri("http://localhost/").body(Empty::new())?;
            let response = sender.send_request(request).await?;
            ensure!(response.status().as_u16() == 200);
            let _ = response.into_body().collect().await?;
            Ok::<_, anyhow::Error>(())
        }));
    }
    for join in joins {
        join.await??;
    }
    Ok(())
}

async fn http1_once(addr: SocketAddr, path: &str) -> Result<(u16, String)> {
    let stream = TcpStream::connect(addr).await?;
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream)).await?;
    tokio::spawn(conn);
    send_http1(&mut sender, path).await
}

async fn send_http1(
    sender: &mut hyper::client::conn::http1::SendRequest<Empty<Bytes>>,
    path: &str,
) -> Result<(u16, String)> {
    let request =
        Request::builder().uri(path).header(hyper::header::HOST, "localhost").body(Empty::new())?;
    let response = sender.send_request(request).await?;
    let status = response.status().as_u16();
    let body = String::from_utf8(response.into_body().collect().await?.to_bytes().to_vec())?;
    Ok((status, body))
}

fn config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 5_000,
        cpu_ms_per_turn: 200,
        memory_limit_bytes: 32 * 1024 * 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_suite_covers_required_metrics() {
        let report = run_http(BenchScale::quick()).expect("http bench");
        assert_eq!(report.suite, "http");
        for name in [
            "http1_keepalive_ms",
            "http2_ms",
            "json_1kb_ms",
            "json_64kb_ms",
            "bytes_64kb_ms",
            "streaming_ms",
            "websocket_echo_ms",
            "sse_ms",
            "http1_concurrency_1_ms",
            "http1_concurrency_10_ms",
            "http1_concurrency_20_ms",
            "http2_concurrency_1_ms",
            "http2_concurrency_10_ms",
            "http2_concurrency_20_ms",
        ] {
            let metric = report
                .metrics
                .iter()
                .find(|metric| metric.name == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert!(!metric.samples.is_empty(), "{name}");
            assert!(metric.p50.is_some(), "{name}");
        }
    }
}
