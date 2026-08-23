use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::Serialize;
use tysel_bench_compare::expected_body;

#[derive(Debug, Parser)]
#[command(about = "Run a windowed HTTP load for sustained-performance diagnostics")]
struct Cli {
    #[arg(long)]
    address: SocketAddr,
    #[arg(long)]
    path: String,
    #[arg(long)]
    response: String,
    #[arg(long, default_value_t = 100)]
    concurrency: usize,
    #[arg(long, default_value_t = 60)]
    duration_seconds: u64,
    #[arg(long, default_value_t = 1000)]
    window_ms: u64,
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadEvidence {
    schema_version: u32,
    address: String,
    path: String,
    response: String,
    concurrency: usize,
    started_at_unix_ms: u128,
    requested_duration_seconds: u64,
    actual_duration_ms: f64,
    total_requests: u64,
    total_errors: u64,
    client_cpu_core_pct: Option<f64>,
    logical_cpus: usize,
    first_third_median_requests_per_second: Option<f64>,
    last_third_median_requests_per_second: Option<f64>,
    sustained_change_pct: Option<f64>,
    windows: Vec<Window>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    index: usize,
    elapsed_ms: f64,
    duration_ms: f64,
    completed_requests: u64,
    errors: u64,
    requests_per_second: f64,
    client_cpu_core_pct: Option<f64>,
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
    ensure!(cli.concurrency > 0, "concurrency must be positive");
    ensure!(cli.duration_seconds > 0, "duration-seconds must be positive");
    ensure!(cli.window_ms > 0, "window-ms must be positive");
    ensure!(cli.path.starts_with('/'), "path must start with '/'");
    let expected = Arc::new(expected_body(&cli.response)?);
    let completed = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let barrier = Arc::new(Barrier::new(cli.concurrency + 1));
    let common_start = Arc::new(OnceLock::new());
    let duration = Duration::from_secs(cli.duration_seconds);
    let address = cli.address;
    let timeout_ms = cli.timeout_ms;
    let mut workers = Vec::with_capacity(cli.concurrency);
    for _ in 0..cli.concurrency {
        let expected = Arc::clone(&expected);
        let completed = Arc::clone(&completed);
        let errors = Arc::clone(&errors);
        let barrier = Arc::clone(&barrier);
        let common_start = Arc::clone(&common_start);
        let path = cli.path.clone();
        workers.push(thread::spawn(move || {
            let mut connection = HttpConnection::connect(address, timeout_ms).ok();
            barrier.wait();
            let deadline = *common_start.get().expect("load start must be initialized") + duration;
            while Instant::now() < deadline {
                let result = connection.as_mut().map_or_else(
                    || Err(anyhow::anyhow!("connection unavailable")),
                    |value| value.request(&path),
                );
                if matches!(result, Ok(ref body) if body == expected.as_slice()) {
                    completed.fetch_add(1, Ordering::Relaxed);
                } else {
                    errors.fetch_add(1, Ordering::Relaxed);
                    connection = HttpConnection::connect(address, timeout_ms).ok();
                    if connection.is_none() {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
            barrier.wait();
        }));
    }
    let started_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    let started = Instant::now();
    common_start.set(started).expect("load start is set once");
    let started_cpu = process_cpu_seconds();
    barrier.wait();
    let window_duration = Duration::from_millis(cli.window_ms);
    let mut windows = Vec::new();
    let mut previous_completed = 0;
    let mut previous_errors = 0;
    let mut previous_elapsed = Duration::ZERO;
    let mut previous_cpu = started_cpu;
    while started.elapsed() < duration {
        let target = (previous_elapsed + window_duration).min(duration);
        thread::sleep(target.saturating_sub(started.elapsed()));
        let elapsed = started.elapsed().min(duration);
        let current_completed = completed.load(Ordering::Relaxed);
        let current_errors = errors.load(Ordering::Relaxed);
        let window_elapsed = elapsed.saturating_sub(previous_elapsed);
        let window_completed = current_completed.saturating_sub(previous_completed);
        let current_cpu = process_cpu_seconds();
        windows.push(Window {
            index: windows.len() + 1,
            elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            duration_ms: window_elapsed.as_secs_f64() * 1000.0,
            completed_requests: window_completed,
            errors: current_errors.saturating_sub(previous_errors),
            requests_per_second: window_completed as f64 / window_elapsed.as_secs_f64(),
            client_cpu_core_pct: cpu_core_pct(previous_cpu, current_cpu, window_elapsed),
        });
        previous_completed = current_completed;
        previous_errors = current_errors;
        previous_elapsed = elapsed;
        previous_cpu = current_cpu;
    }
    barrier.wait();
    for worker in workers {
        worker.join().map_err(|_| anyhow::anyhow!("load worker panicked"))?;
    }
    let (first_third, last_third, sustained_change_pct) = window_trend(&windows);
    let evidence = LoadEvidence {
        schema_version: 1,
        address: cli.address.to_string(),
        path: cli.path,
        response: cli.response,
        concurrency: cli.concurrency,
        started_at_unix_ms,
        requested_duration_seconds: cli.duration_seconds,
        actual_duration_ms: previous_elapsed.as_secs_f64() * 1000.0,
        total_requests: previous_completed,
        total_errors: previous_errors,
        client_cpu_core_pct: cpu_core_pct(started_cpu, previous_cpu, previous_elapsed),
        logical_cpus: thread::available_parallelism().map_or(1, usize::from),
        first_third_median_requests_per_second: first_third,
        last_third_median_requests_per_second: last_third,
        sustained_change_pct,
        windows,
    };
    if let Some(parent) = cli.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut json = serde_json::to_vec_pretty(&evidence)?;
    json.push(b'\n');
    fs::write(&cli.output, json).with_context(|| format!("write {}", cli.output.display()))?;
    ensure!(evidence.total_errors == 0, "load recorded {} errors", evidence.total_errors);
    Ok(())
}

fn window_trend(windows: &[Window]) -> (Option<f64>, Option<f64>, Option<f64>) {
    let third = windows.len() / 3;
    if third == 0 {
        return (None, None, None);
    }
    let median = |slice: &[Window]| {
        let mut values = slice.iter().map(|window| window.requests_per_second).collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let first = median(&windows[..third]);
    let last = median(&windows[windows.len() - third..]);
    let change = (first != 0.0).then_some((last - first) / first * 100.0);
    (Some(first), Some(last), change)
}

fn cpu_core_pct(start: Option<f64>, end: Option<f64>, elapsed: Duration) -> Option<f64> {
    let elapsed = elapsed.as_secs_f64();
    (elapsed > 0.0).then_some((end? - start?) / elapsed * 100.0)
}

#[cfg(target_os = "linux")]
fn process_cpu_seconds() -> Option<f64> {
    use std::process::Command;

    static TICKS_PER_SECOND: OnceLock<f64> = OnceLock::new();
    let ticks_per_second = *TICKS_PER_SECOND.get_or_init(|| {
        Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(100.0)
    });
    let task_entries = fs::read_dir("/proc/self/task").ok()?;
    let mut total_ticks = 0.0;
    let mut sampled_threads = 0;
    for entry in task_entries.flatten() {
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(ticks) = proc_stat_cpu_ticks(&stat) else {
            continue;
        };
        total_ticks += ticks;
        sampled_threads += 1;
    }
    (sampled_threads > 0).then_some(total_ticks / ticks_per_second)
}

#[cfg(not(target_os = "linux"))]
fn process_cpu_seconds() -> Option<f64> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn proc_stat_cpu_ticks(stat: &str) -> Option<f64> {
    let fields = stat.rsplit_once(") ")?.1.split_whitespace().collect::<Vec<_>>();
    let user_ticks = fields.get(11)?.parse::<f64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<f64>().ok()?;
    Some(user_ticks + system_ticks)
}

struct HttpConnection {
    reader: BufReader<TcpStream>,
}

impl HttpConnection {
    fn connect(address: SocketAddr, timeout_ms: u64) -> Result<Self> {
        let timeout = Duration::from_millis(timeout_ms);
        let stream = TcpStream::connect_timeout(&address, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        Ok(Self { reader: BufReader::new(stream) })
    }

    fn request(&mut self, path: &str) -> Result<Vec<u8>> {
        write!(
            self.reader.get_mut(),
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\nAccept: */*\r\n\r\n"
        )?;
        self.reader.get_mut().flush()?;
        let mut status = String::new();
        self.reader.read_line(&mut status)?;
        ensure!(
            status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200"),
            "HTTP status {status:?}"
        );
        let mut content_length = None;
        let mut chunked = false;
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line)?;
            ensure!(!line.is_empty(), "unexpected EOF in HTTP headers");
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>()?);
                }
                if name.eq_ignore_ascii_case("transfer-encoding")
                    && value.to_ascii_lowercase().contains("chunked")
                {
                    chunked = true;
                }
            }
        }
        if let Some(length) = content_length {
            let mut body = vec![0; length];
            self.reader.read_exact(&mut body)?;
            return Ok(body);
        }
        ensure!(chunked, "response has neither content-length nor chunked encoding");
        read_chunked(&mut self.reader)
    }
}

fn read_chunked(reader: &mut BufReader<TcpStream>) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let size =
            usize::from_str_radix(size_line.trim().split(';').next().unwrap_or_default(), 16)?;
        if size == 0 {
            loop {
                let mut trailer = String::new();
                reader.read_line(&mut trailer)?;
                if trailer == "\r\n" || trailer == "\n" {
                    return Ok(body);
                }
            }
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        let mut crlf = [0_u8; 2];
        reader.read_exact(&mut crlf)?;
        ensure!(crlf == *b"\r\n", "invalid chunk terminator");
    }
}

#[cfg(test)]
mod tests {
    use super::{Window, proc_stat_cpu_ticks, window_trend};

    #[test]
    fn trend_compares_first_and_last_thirds() {
        let windows = (0..6)
            .map(|index| Window {
                index,
                elapsed_ms: index as f64,
                duration_ms: 1.0,
                completed_requests: 1,
                errors: 0,
                requests_per_second: [100.0, 110.0, 90.0, 95.0, 80.0, 85.0][index],
                client_cpu_core_pct: None,
            })
            .collect::<Vec<_>>();
        let (first, last, change) = window_trend(&windows);
        assert_eq!(first, Some(110.0));
        assert_eq!(last, Some(85.0));
        assert!((change.unwrap() + 22.727_272_727).abs() < 1e-6);
    }

    #[test]
    fn proc_stat_parser_handles_thread_names_with_spaces() {
        let stat = "42 (load worker 1) R 1 2 3 4 5 6 7 8 9 10 120 30 14";
        assert_eq!(proc_stat_cpu_ticks(stat), Some(150.0));
    }
}
