//! Redis provider microbenchmark.
//!
//! This measures the Rust provider only. Use the `tysel-engine-qjs`
//! `redis_end_to_end_benchmark` example for the public JavaScript runtime path.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redis::aio::ConnectionManager;

const DEFAULT_ITERATIONS: usize = 3_000;
const DEFAULT_ROUNDS: usize = 3;
const MAX_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum GetMode {
    Redis,
    BoundedRedis,
    Tysel,
}

impl GetMode {
    fn label(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::BoundedRedis => "bounded-redis",
            Self::Tysel => "tysel-provider",
        }
    }
}

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
    keys: Vec<String>,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let Ok(client) = redis::Client::open(self.url.as_str()) else {
            return;
        };
        let Ok(mut connection) = client.get_connection() else {
            return;
        };
        let _: redis::RedisResult<u64> = redis::cmd("DEL").arg(&self.keys).query(&mut connection);
    }
}

macro_rules! sequential {
    ($operation:expr, $value_bytes:expr, $mode:expr, $iterations:expr, $future:expr) => {{
        for _ in 0..warmup_iterations($iterations) {
            $future.await;
        }
        let mut samples = Vec::with_capacity($iterations);
        let total_start = Instant::now();
        for _ in 0..$iterations {
            let start = Instant::now();
            $future.await;
            samples.push(start.elapsed());
        }
        measurement(
            $operation,
            $value_bytes,
            $mode,
            $iterations,
            total_start.elapsed(),
            &mut samples,
        )
    }};
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let url =
        std::env::var("TYSEL_REDIS_BENCH_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    let iterations = positive_env("TYSEL_REDIS_BENCH_ITERS", DEFAULT_ITERATIONS);
    let rounds = positive_env("TYSEL_REDIS_BENCH_ROUNDS", DEFAULT_ROUNDS);
    let client = redis::Client::open(url.clone()).expect("valid Redis benchmark URL");
    let manager = ConnectionManager::new(client).await.expect("connect to benchmark Redis");
    tysel_cap_redis::configure(Some(url.clone()), false);

    let suffix = SystemTime::now().duration_since(UNIX_EPOCH).expect("system time").as_nanos();
    let prefix = format!("tysel:bench:{}:{suffix}", std::process::id());
    let keys = [16, 1024, 65_536]
        .into_iter()
        .map(|value_bytes| format!("{prefix}:{value_bytes}"))
        .collect::<Vec<_>>();
    let _cleanup = CleanupGuard { url, keys: keys.clone() };

    let mut results = Vec::new();
    for round in 0..rounds {
        for (value_bytes, key) in [16, 1024, 65_536].into_iter().zip(&keys) {
            let samples = if value_bytes >= 65_536 { iterations.min(750) } else { iterations };
            run_value_size(manager.clone(), key, value_bytes, samples, round, &mut results).await;
        }
    }
    print_results(&results, rounds);
}

async fn run_value_size(
    manager: ConnectionManager,
    key: &str,
    value_bytes: usize,
    iterations: usize,
    round: usize,
    results: &mut Vec<Measurement>,
) {
    let value = "x".repeat(value_bytes);
    let mut seed = manager.clone();
    raw_set(&mut seed, key, &value).await;

    let get_order = match round % 3 {
        0 => [GetMode::Redis, GetMode::BoundedRedis, GetMode::Tysel],
        1 => [GetMode::Tysel, GetMode::Redis, GetMode::BoundedRedis],
        _ => [GetMode::BoundedRedis, GetMode::Tysel, GetMode::Redis],
    };
    for mode in get_order {
        let mut connection = manager.clone();
        let result = match mode {
            GetMode::Redis => {
                sequential!(
                    "get",
                    value_bytes,
                    mode.label(),
                    iterations,
                    raw_get(&mut connection, key)
                )
            }
            GetMode::BoundedRedis => sequential!(
                "get",
                value_bytes,
                mode.label(),
                iterations,
                bounded_raw_get(&mut connection, key)
            ),
            GetMode::Tysel => sequential!("get", value_bytes, mode.label(), iterations, async {
                tysel_cap_redis::get(key).await.expect("Tysel provider GET");
            }),
        };
        results.push(result);
    }

    let set_order = if round.is_multiple_of(2) { [false, true] } else { [true, false] };
    for tysel in set_order {
        let mut connection = manager.clone();
        let result = if tysel {
            sequential!("set", value_bytes, "tysel-provider", iterations, async {
                tysel_cap_redis::set(key, &value, None).await.expect("Tysel provider SET");
            })
        } else {
            sequential!(
                "set",
                value_bytes,
                "redis",
                iterations,
                raw_set(&mut connection, key, &value)
            )
        };
        results.push(result);
    }

    for mode in get_order {
        results.push(concurrent_get(manager.clone(), key, value_bytes, iterations, mode).await);
    }
    for tysel in set_order {
        results.push(
            concurrent_set(manager.clone(), key, &value, value_bytes, iterations, tysel).await,
        );
    }
}

async fn concurrent_get(
    manager: ConnectionManager,
    key: &str,
    value_bytes: usize,
    iterations: usize,
    mode: GetMode,
) -> Measurement {
    let per_worker = iterations.div_ceil(4);
    let actual = per_worker * 4;
    let start = Instant::now();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let mut connection = manager.clone();
        let key = key.to_owned();
        workers.spawn(async move {
            for _ in 0..per_worker {
                match mode {
                    GetMode::Redis => raw_get(&mut connection, &key).await,
                    GetMode::BoundedRedis => bounded_raw_get(&mut connection, &key).await,
                    GetMode::Tysel => {
                        tysel_cap_redis::get(&key).await.expect("concurrent Tysel provider GET");
                    }
                }
            }
        });
    }
    while let Some(result) = workers.join_next().await {
        result.expect("GET benchmark worker");
    }
    throughput("get", value_bytes, c4_label(mode.label()), actual, start.elapsed())
}

async fn concurrent_set(
    manager: ConnectionManager,
    key: &str,
    value: &str,
    value_bytes: usize,
    iterations: usize,
    tysel: bool,
) -> Measurement {
    let per_worker = iterations.div_ceil(4);
    let actual = per_worker * 4;
    let start = Instant::now();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let mut connection = manager.clone();
        let key = key.to_owned();
        let value = value.to_owned();
        workers.spawn(async move {
            for _ in 0..per_worker {
                if tysel {
                    tysel_cap_redis::set(&key, &value, None)
                        .await
                        .expect("concurrent Tysel provider SET");
                } else {
                    raw_set(&mut connection, &key, &value).await;
                }
            }
        });
    }
    while let Some(result) = workers.join_next().await {
        result.expect("SET benchmark worker");
    }
    throughput(
        "set",
        value_bytes,
        if tysel { "tysel-provider-c4" } else { "redis-c4" },
        actual,
        start.elapsed(),
    )
}

async fn raw_get(connection: &mut ConnectionManager, key: &str) {
    redis::cmd("GET").arg(key).query_async::<Vec<u8>>(connection).await.expect("raw GET");
}

async fn bounded_raw_get(connection: &mut ConnectionManager, key: &str) {
    redis::pipe()
        .atomic()
        .cmd("GETRANGE")
        .arg(key)
        .arg(0)
        .arg(MAX_VALUE_BYTES)
        .cmd("EXISTS")
        .arg(key)
        .query_async::<(Vec<u8>, bool)>(connection)
        .await
        .expect("bounded raw GET");
}

async fn raw_set(connection: &mut ConnectionManager, key: &str, value: &str) {
    redis::cmd("SET").arg(key).arg(value).query_async::<String>(connection).await.expect("raw SET");
}

fn measurement(
    operation: &'static str,
    value_bytes: usize,
    mode: &'static str,
    iterations: usize,
    total: Duration,
    samples: &mut [Duration],
) -> Measurement {
    samples.sort_unstable();
    Measurement {
        operation,
        value_bytes,
        mode,
        iterations,
        total_ms: total.as_secs_f64() * 1000.0,
        ops_per_sec: iterations as f64 / total.as_secs_f64(),
        p50_us: Some(percentile(samples, 50).as_secs_f64() * 1_000_000.0),
        p95_us: Some(percentile(samples, 95).as_secs_f64() * 1_000_000.0),
        p99_us: Some(percentile(samples, 99).as_secs_f64() * 1_000_000.0),
    }
}

fn throughput(
    operation: &'static str,
    value_bytes: usize,
    mode: &'static str,
    iterations: usize,
    total: Duration,
) -> Measurement {
    Measurement {
        operation,
        value_bytes,
        mode,
        iterations,
        total_ms: total.as_secs_f64() * 1000.0,
        ops_per_sec: iterations as f64 / total.as_secs_f64(),
        p50_us: None,
        p95_us: None,
        p99_us: None,
    }
}

fn print_results(results: &[Measurement], rounds: usize) {
    println!(
        "kind,round,operation,value_bytes,mode,iterations,total_ms,ops_per_sec,p50_us,p95_us,p99_us"
    );
    for (index, result) in results.iter().enumerate() {
        print_measurement("raw", index / (results.len() / rounds) + 1, result);
    }

    let mut groups: BTreeMap<(&str, usize, &str), Vec<&Measurement>> = BTreeMap::new();
    for result in results {
        groups.entry((result.operation, result.value_bytes, result.mode)).or_default().push(result);
    }
    for ((operation, value_bytes, mode), group) in groups {
        let summary = Measurement {
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
        print_measurement("median", 0, &summary);
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
    (iterations / 10).clamp(50, 500).min(iterations)
}

fn c4_label(mode: &str) -> &'static str {
    match mode {
        "redis" => "redis-c4",
        "bounded-redis" => "bounded-redis-c4",
        "tysel-provider" => "tysel-provider-c4",
        _ => unreachable!("known benchmark mode"),
    }
}
