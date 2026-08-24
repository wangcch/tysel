use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result, ensure};
use clap::Parser;
use rquickjs::{ArrayBuffer, Context, Function, Object, Runtime, TypedArray, Value};
use serde::Serialize;
use tysel_engine::{HttpRequest, IsolateConfig};
use tysel_engine_qjs::{IncomingHttp, IsolatePool, OutgoingHttpBody};

const PAYLOAD_BYTES: usize = 65_536;
const WEB_API: &str = include_str!("../../../../runtime-js/web-api/runtime.js");

const HANDLER: &str = r#"
export default {
  fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/health") {
      return new Response("ok");
    }
    if (path === "/string") {
      return new Response("x".repeat(65536));
    }
    if (path === "/typed") {
      const body = new Uint8Array(65536);
      body.fill(120);
      return new Response(body);
    }
    if (path === "/json") {
      return Response.json({ payload: "b".repeat(65536) });
    }
    return new Response("not found", { status: 404 });
  },
};
"#;

const WEB_HANDLER: &str = r#"
(url) => {
  const request = new Request(url, { method: "GET", bodyStream: true, headers: {} });
  const path = new URL(request.url).pathname;
  if (path === "/health") {
    return new Response("ok");
  }
  if (path === "/string") {
    return new Response("x".repeat(65536));
  }
  if (path === "/typed") {
    const body = new Uint8Array(65536);
    body.fill(120);
    return new Response(body);
  }
  if (path === "/json") {
    return Response.json({ payload: "b".repeat(65536) });
  }
  return new Response("not found", { status: 404 });
}
"#;

#[derive(Debug, Parser)]
#[command(about = "Separate bare QuickJS work from Tysel fetch-boundary cost")]
struct Cli {
    #[arg(long, default_value_t = 200)]
    warmup_iterations: usize,
    #[arg(long, default_value_t = 2_000)]
    iterations: usize,
    #[arg(long, default_value = "target/benchmark-comparison/qjs-boundary-diagnostic.json")]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence {
    schema_version: u32,
    quickjs_adapter: &'static str,
    payload_bytes: usize,
    warmup_iterations: usize,
    iterations: usize,
    measurements: Vec<Measurement>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Measurement {
    workload: &'static str,
    layer: &'static str,
    median_ns_per_operation: f64,
    operations_per_second: f64,
    checksum: usize,
}

#[derive(Clone, Copy)]
enum Workload {
    Health,
    String,
    Typed,
    Json,
}

impl Workload {
    const ALL: [Self; 4] = [Self::Health, Self::String, Self::Typed, Self::Json];

    fn name(self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::String => "string-64k",
            Self::Typed => "typed-64k",
            Self::Json => "json-64k",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Health => "/health",
            Self::String => "/string",
            Self::Typed => "/typed",
            Self::Json => "/json",
        }
    }

    fn engine_script(self) -> &'static str {
        match self {
            Self::Health => "() => 'ok'.length",
            Self::String => "() => 'x'.repeat(65536).length",
            Self::Typed => {
                "() => { const body = new Uint8Array(65536); body.fill(120); return body.byteLength; }"
            }
            Self::Json => "() => JSON.stringify({ payload: 'b'.repeat(65536) }).length",
        }
    }

    fn extraction_script(self) -> &'static str {
        match self {
            Self::Health => "() => 'ok'",
            Self::String => "() => 'x'.repeat(65536)",
            Self::Typed => {
                "() => { const body = new Uint8Array(65536); body.fill(120); return body; }"
            }
            Self::Json => "() => JSON.stringify({ payload: 'b'.repeat(65536) })",
        }
    }

    fn expected_len(self) -> usize {
        match self {
            Self::Health => 2,
            Self::String | Self::Typed => PAYLOAD_BYTES,
            Self::Json => PAYLOAD_BYTES + r#"{"payload":""#.len() + 2,
        }
    }
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    ensure!(cli.warmup_iterations > 0, "warmup-iterations must be positive");
    ensure!(cli.iterations > 0, "iterations must be positive");

    let mut measurements = Vec::new();
    for workload in Workload::ALL {
        measurements.push(measure_bare_engine(workload, &cli)?);
        measurements.push(measure_bare_extraction(workload, &cli)?);
        measurements.push(measure_web_api_boundary(workload, &cli)?);
        measurements.push(measure_tysel_boundary(workload, &cli)?);
    }

    let evidence = Evidence {
        schema_version: 1,
        quickjs_adapter: tysel_engine_qjs::QUICKJS_ADAPTER_ID,
        payload_bytes: PAYLOAD_BYTES,
        warmup_iterations: cli.warmup_iterations,
        iterations: cli.iterations,
        measurements,
    };
    if let Some(parent) = cli.output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut json = serde_json::to_vec_pretty(&evidence)?;
    json.push(b'\n');
    fs::write(&cli.output, json).with_context(|| format!("write {}", cli.output.display()))?;
    println!("QuickJS boundary diagnostic {}", cli.output.display());
    for item in &evidence.measurements {
        println!(
            "{:<10} {:<18} {:>10.0} ns/op {:>10.0} ops/s",
            item.workload, item.layer, item.median_ns_per_operation, item.operations_per_second
        );
    }
    Ok(())
}

fn measure_bare_engine(workload: Workload, cli: &Cli) -> Result<Measurement> {
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let samples = context.with(|ctx| -> Result<Vec<(u128, usize)>> {
        let function: Function = ctx.eval(workload.engine_script())?;
        sample(cli, || {
            let length: usize = function.call(())?;
            ensure!(length == workload.expected_len(), "unexpected engine result length {length}");
            Ok(length)
        })
    })?;
    Ok(measurement(workload, "bare-engine", samples))
}

fn measure_bare_extraction(workload: Workload, cli: &Cli) -> Result<Measurement> {
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let samples = context.with(|ctx| -> Result<Vec<(u128, usize)>> {
        let function: Function = ctx.eval(workload.extraction_script())?;
        sample(cli, || {
            let value: Value = function.call(())?;
            let bytes = extract_bytes(value)?;
            ensure!(bytes.len() == workload.expected_len(), "unexpected extracted length");
            Ok(black_box(bytes).len())
        })
    })?;
    Ok(measurement(workload, "bare-engine+copy", samples))
}

fn measure_web_api_boundary(workload: Workload, cli: &Cli) -> Result<Measurement> {
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let samples = context.with(|ctx| -> Result<Vec<(u128, usize)>> {
        ctx.eval::<(), _>(WEB_API)?;
        let function: Function = ctx.eval(WEB_HANDLER)?;
        sample(cli, || {
            let response: Object =
                function.call((format!("http://tysel.local{}", workload.path()),))?;
            let status: i32 = response.get("status")?;
            ensure!(status == 200, "unexpected Web API response status {status}");
            let headers: Object = response.get("headers")?;
            let map: Object = headers.get("_map")?;
            let mut header_bytes = 0usize;
            for property in map.props::<String, String>() {
                let (name, value) = property?;
                header_bytes = header_bytes.wrapping_add(name.len()).wrapping_add(value.len());
            }
            let body: Value = response.get("body")?;
            let bytes = extract_bytes(body)?;
            ensure!(bytes.len() == workload.expected_len(), "unexpected Web API body length");
            Ok(black_box(bytes).len().wrapping_add(header_bytes))
        })
    })?;
    Ok(measurement(workload, "web-api+copy", samples))
}

fn measure_tysel_boundary(workload: Workload, cli: &Cli) -> Result<Measurement> {
    let pool = IsolatePool::spawn(1, HANDLER, isolate_config())?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let samples = sample(cli, || {
        let request = IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: format!("http://tysel.local{}", workload.path()),
            headers: Vec::new(),
            body: Vec::new(),
            request_id: 0,
        });
        let (head, body) = runtime.block_on(pool.dispatch_response(request))?;
        ensure!(head.status == 200, "unexpected response status {}", head.status);
        let OutgoingHttpBody::Buffered(bytes) = body else {
            anyhow::bail!("diagnostic response was not buffered");
        };
        ensure!(bytes.len() == workload.expected_len(), "unexpected Tysel response length");
        Ok(black_box(bytes).len())
    })?;
    Ok(measurement(workload, "tysel-fetch-boundary", samples))
}

fn sample(cli: &Cli, mut operation: impl FnMut() -> Result<usize>) -> Result<Vec<(u128, usize)>> {
    for _ in 0..cli.warmup_iterations {
        black_box(operation()?);
    }
    let mut samples = Vec::with_capacity(cli.iterations);
    for _ in 0..cli.iterations {
        let started = Instant::now();
        let checksum = operation()?;
        samples.push((started.elapsed().as_nanos(), checksum));
    }
    Ok(samples)
}

fn measurement(
    workload: Workload,
    layer: &'static str,
    mut samples: Vec<(u128, usize)>,
) -> Measurement {
    samples.sort_unstable_by_key(|sample| sample.0);
    let median = samples[samples.len() / 2].0 as f64;
    let checksum = samples.iter().fold(0usize, |sum, sample| sum.wrapping_add(sample.1));
    Measurement {
        workload: workload.name(),
        layer,
        median_ns_per_operation: median,
        operations_per_second: 1_000_000_000.0 / median,
        checksum,
    }
}

fn extract_bytes(value: Value<'_>) -> Result<Vec<u8>> {
    if let Some(text) = value.as_string() {
        return Ok(text.to_string()?.into_bytes());
    }
    if let Ok(view) = TypedArray::<u8>::from_value(value.clone()) {
        return view.as_bytes().map(ToOwned::to_owned).context("detached Uint8Array");
    }
    if let Some(buffer) = ArrayBuffer::from_value(value) {
        return buffer.as_bytes().map(ToOwned::to_owned).context("detached ArrayBuffer");
    }
    anyhow::bail!("unsupported bare QuickJS result")
}

fn isolate_config() -> IsolateConfig {
    IsolateConfig {
        memory_limit_bytes: 32 * 1024 * 1024,
        cpu_ms_per_turn: 2_000,
        request_timeout_ms: 2_000,
    }
}
