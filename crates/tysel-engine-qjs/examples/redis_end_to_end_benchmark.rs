//! End-to-end `tysel.redis` benchmark through QuickJS and the host I/O reactor.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redis::aio::ConnectionManager;
use tysel_engine::{HttpRequest, IsolateConfig};
use tysel_engine_qjs::{IsolatePool, configure_execution_profile, configure_redis};

const DEFAULT_ITERATIONS: usize = 1_000;
const DEFAULT_ROUNDS: usize = 3;

#[derive(Clone)]
struct Measurement {
    operation: &'static str,
    value_bytes: usize,
    mode: &'static str,
    iterations: usize,
    total_ms: f64,
    ops_per_sec: f64,
    p50_us: Option<f64>,
    p95_us: Option<f64>,
    p99_us: Option<f64>,
}

struct CleanupGuard {
    url: String,
    key: String,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let Ok(client) = redis::Client::open(self.url.as_str()) else {
            return;
        };
        let Ok(mut connection) = client.get_connection() else {
            return;
        };
        let _: redis::RedisResult<u64> = redis::cmd("DEL").arg(&self.key).query(&mut connection);
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let url =
        std::env::var("TYSEL_REDIS_BENCH_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    let iterations = positive_env("TYSEL_REDIS_E2E_BENCH_ITERS", DEFAULT_ITERATIONS);
    let rounds = positive_env("TYSEL_REDIS_BENCH_ROUNDS", DEFAULT_ROUNDS);
    configure_execution_profile("service");
    configure_redis(Some(url.clone()), false);

    let suffix = SystemTime::now().duration_since(UNIX_EPOCH).expect("system time").as_nanos();
    let prefix = format!("tysel:e2e-bench:{}:{suffix}", std::process::id());
    let mut results = Vec::new();
    for round in 0..rounds {
        for value_bytes in [16, 1024, 65_536] {
            let key = format!("{prefix}:{value_bytes}");
            let _cleanup = CleanupGuard { url: url.clone(), key: key.clone() };
            seed(&url, &key, value_bytes).await;
            let pool = Arc::new(
                IsolatePool::spawn(4, &handler_source(&key, value_bytes), config())
                    .expect("spawn QuickJS isolate pool"),
            );
            let samples = if value_bytes >= 65_536 { iterations.min(250) } else { iterations };
            let modes = if round.is_multiple_of(2) {
                ["runtime-noop", "tysel-e2e"]
            } else {
                ["tysel-e2e", "runtime-noop"]
            };
            for operation in ["get", "set"] {
                for mode in modes {
                    let path = path(operation, mode);
                    results.push(
                        sequential(pool.clone(), path, operation, value_bytes, mode, samples).await,
                    );
                }
                for mode in modes {
                    let path = path(operation, mode);
                    results.push(
                        concurrent(pool.clone(), path, operation, value_bytes, mode, samples).await,
                    );
                }
            }
        }
    }
    print_results(&results, rounds);
}

async fn sequential(
    pool: Arc<IsolatePool>,
    path: &'static str,
    operation: &'static str,
    value_bytes: usize,
    mode: &'static str,
    iterations: usize,
) -> Measurement {
    for _ in 0..warmup_iterations(iterations) {
        dispatch(&pool, path).await;
    }
    let mut samples = Vec::with_capacity(iterations);
    let total_start = Instant::now();
    for _ in 0..iterations {
        let start = Instant::now();
        dispatch(&pool, path).await;
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let total = total_start.elapsed();
    Measurement {
        operation,
        value_bytes,
        mode,
        iterations,
        total_ms: total.as_secs_f64() * 1000.0,
        ops_per_sec: iterations as f64 / total.as_secs_f64(),
        p50_us: Some(percentile(&samples, 50).as_secs_f64() * 1_000_000.0),
        p95_us: Some(percentile(&samples, 95).as_secs_f64() * 1_000_000.0),
        p99_us: Some(percentile(&samples, 99).as_secs_f64() * 1_000_000.0),
    }
}

async fn concurrent(
    pool: Arc<IsolatePool>,
    path: &'static str,
    operation: &'static str,
    value_bytes: usize,
    mode: &'static str,
    iterations: usize,
) -> Measurement {
    let per_worker = iterations.div_ceil(4);
    let actual = per_worker * 4;
    let start = Instant::now();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let pool = pool.clone();
        workers.spawn(async move {
            for _ in 0..per_worker {
                dispatch(&pool, path).await;
            }
        });
    }
    while let Some(result) = workers.join_next().await {
        result.expect("end-to-end benchmark worker");
    }
    let total = start.elapsed();
    Measurement {
        operation,
        value_bytes,
        mode: c4_label(mode),
        iterations: actual,
        total_ms: total.as_secs_f64() * 1000.0,
        ops_per_sec: actual as f64 / total.as_secs_f64(),
        p50_us: None,
        p95_us: None,
        p99_us: None,
    }
}

async fn dispatch(pool: &IsolatePool, path: &str) {
    let (head, mut body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: format!("http://tysel.local{path}"),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch benchmark request");
    assert!(matches!(head.status, 200 | 204), "unexpected status {}", head.status);
    while body.recv().await.is_some() {}
}

async fn seed(url: &str, key: &str, value_bytes: usize) {
    let client = redis::Client::open(url).expect("valid Redis benchmark URL");
    let mut manager = ConnectionManager::new(client).await.expect("connect to benchmark Redis");
    redis::cmd("SET")
        .arg(key)
        .arg("x".repeat(value_bytes))
        .query_async::<String>(&mut manager)
        .await
        .expect("seed end-to-end benchmark value");
}

fn handler_source(key: &str, value_bytes: usize) -> String {
    let key = serde_json::to_string(key).expect("serialize benchmark key");
    let value = serde_json::to_string(&"x".repeat(value_bytes)).expect("serialize benchmark value");
    format!(
        r#"
const key = {key};
const value = {value};
export default {{
  async fetch(request) {{
    const path = new URL(request.url).pathname;
    if (path === "/redis-get") return new Response((await tysel.redis.get(key)) ?? "");
    if (path === "/redis-set") {{
      await tysel.redis.set(key, value);
      return new Response(null, {{ status: 204 }});
    }}
    if (path === "/noop-get") return new Response(value);
    if (path === "/noop-set") return new Response(null, {{ status: 204 }});
    return new Response("not found", {{ status: 404 }});
  }}
}};
"#
    )
}

fn config() -> IsolateConfig {
    IsolateConfig {
        memory_limit_bytes: 64 * 1024 * 1024,
        cpu_ms_per_turn: 100,
        request_timeout_ms: 10_000,
    }
}

fn path(operation: &str, mode: &str) -> &'static str {
    match (operation, mode) {
        ("get", "runtime-noop") => "/noop-get",
        ("get", "tysel-e2e") => "/redis-get",
        ("set", "runtime-noop") => "/noop-set",
        ("set", "tysel-e2e") => "/redis-set",
        _ => unreachable!("known benchmark operation and mode"),
    }
}

fn print_results(results: &[Measurement], rounds: usize) {
    println!(
        "kind,round,operation,value_bytes,mode,iterations,total_ms,ops_per_sec,p50_us,p95_us,p99_us"
    );
    let per_round = results.len() / rounds;
    for (index, result) in results.iter().enumerate() {
        print_measurement("raw", index / per_round + 1, result);
    }
    let mut groups: BTreeMap<(&str, usize, &str), Vec<&Measurement>> = BTreeMap::new();
    for result in results {
        groups.entry((result.operation, result.value_bytes, result.mode)).or_default().push(result);
    }
    for ((operation, value_bytes, mode), group) in groups {
        let item = Measurement {
            operation,
            value_bytes,
            mode,
            iterations: median_usize(group.iter().map(|item| item.iterations)),
            total_ms: median_f64(group.iter().map(|item| item.total_ms)),
            ops_per_sec: median_f64(group.iter().map(|item| item.ops_per_sec)),
            p50_us: median_option(group.iter().filter_map(|item| item.p50_us)),
            p95_us: median_option(group.iter().filter_map(|item| item.p95_us)),
            p99_us: median_option(group.iter().filter_map(|item| item.p99_us)),
        };
        print_measurement("median", 0, &item);
    }
}

fn print_measurement(kind: &str, round: usize, item: &Measurement) {
    println!(
        "{kind},{round},{},{},{},{},{:.3},{:.1},{},{},{}",
        item.operation,
        item.value_bytes,
        item.mode,
        item.iterations,
        item.total_ms,
        item.ops_per_sec,
        format_option(item.p50_us),
        format_option(item.p95_us),
        format_option(item.p99_us),
    );
}

fn median_f64(values: impl Iterator<Item = f64>) -> f64 {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn median_usize(values: impl Iterator<Item = usize>) -> usize {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}

fn median_option(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.collect::<Vec<_>>();
    (!values.is_empty()).then(|| median_f64(values.into_iter()))
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    samples[(samples.len() - 1) * percentile / 100]
}

fn format_option(value: Option<f64>) -> String {
    value.map(|value| format!("{value:.1}")).unwrap_or_default()
}

fn positive_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn warmup_iterations(iterations: usize) -> usize {
    (iterations / 10).clamp(25, 200).min(iterations)
}

fn c4_label(mode: &str) -> &'static str {
    match mode {
        "runtime-noop" => "runtime-noop-c4",
        "tysel-e2e" => "tysel-e2e-c4",
        _ => unreachable!("known benchmark mode"),
    }
}
