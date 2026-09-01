use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};
use clap::Parser;
use tysel_bench_compare::{
    COMPARISON_SCHEMA_VERSION, CommandSpec, ComparisonEvidence, HttpRound, HttpWorkloadEvidence,
    MemoryMeasurement, RuntimeEvidence, RuntimeSpec, ToolchainEvidence, ToolchainSpec,
    benchmark_system, distribution, evidence_shell_command, expected_body, git_state, load_matrix,
    load_runtime_lock, now_unix_ms, process_memory_kb, quick_matrix, render_markdown,
    resolve_executable, sha256_file, workspace_root,
};

#[derive(Debug, Parser)]
#[command(about = "Compare Tysel, Node.js, Bun, and Deno with matched external workloads")]
struct Cli {
    #[arg(long, default_value = "benchmarks/comparison/matrix.toml")]
    matrix: PathBuf,
    #[arg(long, default_value = "benchmarks/comparison/runtimes.lock.json")]
    runtimes: PathBuf,
    #[arg(long, default_value = "target/benchmark-comparison/comparison-v1.json")]
    output: PathBuf,
    #[arg(long)]
    quick: bool,
    #[arg(long)]
    allow_missing: bool,
    #[arg(long)]
    skip_prepare: bool,
    #[arg(long, value_delimiter = ',')]
    runtime: Vec<String>,
    #[arg(long, default_value_t = 1)]
    order_seed: u64,
}

#[derive(Debug)]
struct RuntimeUnavailable(String);

impl fmt::Display for RuntimeUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeUnavailable {}

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
    let root = workspace_root();
    let matrix_path = resolve_path(&root, &cli.matrix);
    let runtime_path = resolve_path(&root, &cli.runtimes);
    let output_path = resolve_path(&root, &cli.output);
    let matrix = load_matrix(&matrix_path)?;
    let matrix = if cli.quick { quick_matrix(matrix) } else { matrix };
    let mut runtime_lock = load_runtime_lock(&runtime_path)?;
    if !cli.runtime.is_empty() {
        runtime_lock.runtimes.retain(|runtime| cli.runtime.contains(&runtime.id));
        ensure!(!runtime_lock.runtimes.is_empty(), "runtime filter matched nothing");
    }
    if !cli.skip_prepare {
        for command in &runtime_lock.prepare {
            run_prepare(&root, command).context("prepare shared benchmark sources")?;
        }
        for runtime in &runtime_lock.runtimes {
            for command in &runtime.prepare {
                run_prepare(&root, command).with_context(|| format!("prepare {}", runtime.id))?;
            }
        }
    }
    let toolchains = runtime_lock
        .toolchains
        .iter()
        .map(|toolchain| toolchain_evidence(&root, toolchain))
        .collect::<Result<Vec<_>>>()?;
    rotate_order(&mut runtime_lock.runtimes, cli.order_seed);

    let (source_commit, workspace_dirty) = git_state(&root)?;
    let generated_at_unix_ms = now_unix_ms()?;
    let run_id = format!("{}-{}-{}", generated_at_unix_ms, std::env::consts::ARCH, cli.order_seed);
    let mut runtimes = Vec::new();
    for (execution_order, runtime) in runtime_lock.runtimes.iter().enumerate() {
        eprintln!(
            "compare {} ({}/{})",
            runtime.id,
            execution_order + 1,
            runtime_lock.runtimes.len()
        );
        let result = measure_runtime(&root, runtime, &matrix, &cli, execution_order + 1);
        match result {
            Ok(value) => runtimes.push(value),
            Err(error)
                if cli.allow_missing && error.downcast_ref::<RuntimeUnavailable>().is_some() =>
            {
                eprintln!("skip {}: {error:#}", runtime.id);
                runtimes.push(RuntimeEvidence {
                    id: runtime.id.clone(),
                    expected_version: runtime.expected_version.clone(),
                    actual_version: None,
                    build_mode: runtime.build_mode.clone(),
                    executable: None,
                    executable_sha256: None,
                    status: "unavailable".into(),
                    reason: Some(format!("{error:#}")),
                    execution_order: execution_order + 1,
                    startup_ms: None,
                    idle_memory: None,
                    workloads: Vec::new(),
                });
            }
            Err(error) => return Err(error).with_context(|| format!("measure {}", runtime.id)),
        }
    }

    let evidence = ComparisonEvidence {
        schema_version: COMPARISON_SCHEMA_VERSION,
        run_id,
        generated_at_unix_ms,
        source_commit,
        workspace_dirty,
        command: evidence_shell_command(),
        matrix: relative_label(&root, &matrix_path),
        runtime_lock: relative_label(&root, &runtime_path),
        quick: cli.quick,
        order_seed: cli.order_seed,
        system: benchmark_system(),
        toolchains,
        runtimes,
    };
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(&evidence)?;
    bytes.push(b'\n');
    fs::write(&output_path, bytes).with_context(|| format!("write {}", output_path.display()))?;
    let report_path = output_path.with_extension("md");
    fs::write(&report_path, render_markdown(&evidence))
        .with_context(|| format!("write {}", report_path.display()))?;
    println!("Evidence  {}", output_path.display());
    println!("Report    {}", report_path.display());
    Ok(())
}

fn measure_runtime(
    root: &Path,
    runtime: &RuntimeSpec,
    matrix: &tysel_bench_compare::Matrix,
    cli: &Cli,
    execution_order: usize,
) -> Result<RuntimeEvidence> {
    let executable = resolve_executable(root, &runtime.executable).ok_or_else(|| {
        RuntimeUnavailable(format!("executable not found: {}", runtime.executable))
    })?;
    let actual_version = runtime_version(root, runtime, &executable)?;
    if !actual_version.contains(&runtime.expected_version) {
        return Err(RuntimeUnavailable(format!(
            "{} version mismatch: expected {}, got {}",
            runtime.id, runtime.expected_version, actual_version
        ))
        .into());
    }
    let executable_sha256 = sha256_file(&executable)?;

    for _ in 0..matrix.measurement.startup_warmups {
        let (mut server, _, _) =
            spawn_ready(root, runtime, &executable, matrix.measurement.request_timeout_ms)?;
        stop_server(&mut server);
    }
    let mut startup_samples = Vec::with_capacity(matrix.measurement.startup_samples);
    for _ in 0..matrix.measurement.startup_samples {
        let started = Instant::now();
        let (mut server, _, _) =
            spawn_ready(root, runtime, &executable, matrix.measurement.request_timeout_ms)?;
        startup_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        stop_server(&mut server);
    }

    let (mut server, address, pid) =
        spawn_ready(root, runtime, &executable, matrix.measurement.request_timeout_ms)?;
    thread::sleep(Duration::from_millis(matrix.measurement.idle_settle_ms));
    let idle_memory = process_tree_memory(pid)?;
    let mut workloads = Vec::new();
    for workload in &matrix.workloads {
        let expected = expected_body(&workload.response)?;
        let body = one_request(address, &workload.path, matrix.measurement.request_timeout_ms)?;
        ensure!(body == expected, "{} {} response contract mismatch", runtime.id, workload.id);
        for &concurrency in &matrix.measurement.concurrency {
            let warmup = matrix.measurement.http_warmup_requests.max(concurrency);
            let warmup_result = run_http_round(
                address,
                pid,
                &workload.path,
                &expected,
                RoundLimit::Requests(warmup),
                concurrency,
                matrix.measurement.request_timeout_ms,
            )?;
            ensure!(warmup_result.errors == 0, "{} {} warmup failed", runtime.id, workload.id);
            let mut rounds = Vec::with_capacity(matrix.measurement.http_rounds);
            for _ in 0..matrix.measurement.http_rounds {
                rounds.push(run_http_round(
                    address,
                    pid,
                    &workload.path,
                    &expected,
                    RoundLimit::Duration(Duration::from_millis(
                        matrix.measurement.http_round_duration_ms,
                    )),
                    concurrency,
                    matrix.measurement.request_timeout_ms,
                )?);
            }
            let errors = rounds.iter().map(|round| round.errors).sum();
            let rps = rounds.iter().map(|round| round.requests_per_second).collect();
            let latency =
                rounds.iter().flat_map(|round| round.latency_ms.iter().copied()).collect();
            workloads.push(HttpWorkloadEvidence {
                id: workload.id.clone(),
                path: workload.path.clone(),
                concurrency,
                round_duration_ms: matrix.measurement.http_round_duration_ms,
                rounds,
                requests_per_second: distribution(rps, cli.order_seed ^ concurrency as u64),
                latency_ms: distribution(latency, cli.order_seed ^ 0x9e37_79b9),
                errors,
            });
        }
    }
    stop_server(&mut server);
    Ok(RuntimeEvidence {
        id: runtime.id.clone(),
        expected_version: runtime.expected_version.clone(),
        actual_version: Some(actual_version),
        build_mode: runtime.build_mode.clone(),
        executable: Some(executable.display().to_string()),
        executable_sha256: Some(executable_sha256),
        status: "measured".into(),
        reason: None,
        execution_order,
        startup_ms: Some(distribution(startup_samples, cli.order_seed ^ 0x517c_c1b7)),
        idle_memory: Some(idle_memory),
        workloads,
    })
}

fn toolchain_evidence(root: &Path, toolchain: &ToolchainSpec) -> Result<ToolchainEvidence> {
    let executable =
        resolve_executable(root, &toolchain.version_command.executable).ok_or_else(|| {
            anyhow!("toolchain executable not found: {}", toolchain.version_command.executable)
        })?;
    let output = Command::new(&executable)
        .args(&toolchain.version_command.args)
        .current_dir(root)
        .output()
        .with_context(|| format!("query {} version", toolchain.id))?;
    ensure!(output.status.success(), "{} version command failed", toolchain.id);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let actual_version = format!("{} {}", stdout.trim(), stderr.trim()).trim().to_owned();
    ensure!(
        actual_version.contains(&toolchain.expected_version),
        "{} version mismatch: expected {}, got {}",
        toolchain.id,
        toolchain.expected_version,
        actual_version
    );
    Ok(ToolchainEvidence {
        id: toolchain.id.clone(),
        expected_version: toolchain.expected_version.clone(),
        actual_version,
        executable: executable.display().to_string(),
        executable_sha256: sha256_file(&executable)?,
    })
}

fn run_prepare(root: &Path, command: &CommandSpec) -> Result<()> {
    let executable = resolve_executable(root, &command.executable)
        .ok_or_else(|| anyhow!("prepare executable not found: {}", command.executable))?;
    let status = Command::new(&executable)
        .args(&command.args)
        .current_dir(root)
        .status()
        .with_context(|| format!("run {}", executable.display()))?;
    ensure!(status.success(), "prepare command failed: {}", executable.display());
    Ok(())
}

fn runtime_version(root: &Path, runtime: &RuntimeSpec, executable: &Path) -> Result<String> {
    let (version_executable, args) = match &runtime.version_command {
        Some(command) => (
            resolve_executable(root, &command.executable)
                .ok_or_else(|| anyhow!("version executable not found: {}", command.executable))?,
            command.args.as_slice(),
        ),
        None => (executable.to_owned(), &["--version".to_owned()][..]),
    };
    let output = Command::new(&version_executable)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("run {} --version", runtime.id))?;
    ensure!(output.status.success(), "{} version command failed", runtime.id);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let version = format!("{} {}", stdout.trim(), stderr.trim()).trim().to_owned();
    ensure!(!version.is_empty(), "{} version command returned no text", runtime.id);
    Ok(version)
}

fn spawn_ready(
    root: &Path,
    runtime: &RuntimeSpec,
    executable: &Path,
    timeout_ms: u64,
) -> Result<(Child, SocketAddr, u32)> {
    let mut child = Command::new(executable)
        .args(&runtime.args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {}", runtime.id))?;
    let pid = child.id();
    let stdout = child.stdout.take().context("runtime stdout is not piped")?;
    let prefix = runtime.readiness_prefix.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut announced = false;
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if !announced && let Some(value) = line.strip_prefix(&prefix) {
                let parsed = value.trim().parse::<SocketAddr>().map_err(anyhow::Error::from);
                let _ = tx.send(parsed);
                announced = true;
            }
        }
        if !announced {
            let _ = tx.send(Err(anyhow!("runtime exited before readiness announcement")));
        }
    });
    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(address)) => Ok((child, address, pid)),
        Ok(Err(error)) => {
            let status = child
                .try_wait()
                .ok()
                .flatten()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "still running".to_owned());
            stop_server(&mut child);
            Err(error).with_context(|| format!("{} readiness failed; status={status}", runtime.id))
        }
        Err(_) => {
            let status = child
                .try_wait()
                .ok()
                .flatten()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "still running".to_owned());
            stop_server(&mut child);
            Err(anyhow!("timed out waiting for {} readiness; status={status}", runtime.id))
        }
    }
}

fn stop_server(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn process_tree_memory(root_pid: u32) -> Result<MemoryMeasurement> {
    let pids = process_tree_pids(root_pid)?;
    let mut value_kb = 0_u64;
    let mut kind = None;
    for pid in &pids {
        if let Ok((value, measured_kind)) = process_memory_kb(*pid) {
            value_kb = value_kb.saturating_add(value);
            kind = Some(measured_kind);
        }
    }
    let kind = kind.context("no process memory sample available")?;
    Ok(MemoryMeasurement { value_kb, kind: kind.into(), process_count: pids.len() })
}

fn process_tree_pids(root_pid: u32) -> Result<Vec<u32>> {
    #[cfg(target_os = "linux")]
    {
        let mut out = vec![root_pid];
        let mut index = 0;
        while index < out.len() {
            let pid = out[index];
            let path = format!("/proc/{pid}/task/{pid}/children");
            if let Ok(text) = fs::read_to_string(path) {
                for child in text.split_whitespace().filter_map(|value| value.parse::<u32>().ok()) {
                    if !out.contains(&child) {
                        out.push(child);
                    }
                }
            }
            index += 1;
        }
        Ok(out)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(vec![root_pid])
    }
}

fn one_request(address: SocketAddr, path: &str, timeout_ms: u64) -> Result<Vec<u8>> {
    let mut connection = HttpConnection::connect(address, timeout_ms)?;
    connection.request(path)
}

fn run_http_round(
    address: SocketAddr,
    server_pid: u32,
    path: &str,
    expected: &[u8],
    limit: RoundLimit,
    concurrency: usize,
    timeout_ms: u64,
) -> Result<HttpRound> {
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler_flag = Arc::clone(&sampling);
    let memory_sampler = thread::spawn(move || {
        let mut peak = process_tree_memory(server_pid).ok();
        while sampler_flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
            if let Ok(sample) = process_tree_memory(server_pid)
                && peak.as_ref().is_none_or(|current| sample.value_kb > current.value_kb)
            {
                peak = Some(sample);
            }
        }
        peak
    });
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    for worker in 0..concurrency {
        let worker_limit = match limit {
            RoundLimit::Requests(requests) => RoundLimit::Requests(
                requests / concurrency + usize::from(worker < requests % concurrency),
            ),
            RoundLimit::Duration(duration) => RoundLimit::Duration(duration),
        };
        let path = path.to_owned();
        let expected = expected.to_vec();
        let worker_barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let capacity = match worker_limit {
                RoundLimit::Requests(requests) => requests,
                RoundLimit::Duration(_) => 1_024,
            };
            let mut latencies = Vec::with_capacity(capacity);
            let mut errors = 0;
            let mut connection = HttpConnection::connect(address, timeout_ms).ok();
            worker_barrier.wait();
            let worker_started = Instant::now();
            let mut attempted = 0_usize;
            loop {
                let should_continue = match worker_limit {
                    RoundLimit::Requests(requests) => attempted < requests,
                    RoundLimit::Duration(duration) => worker_started.elapsed() < duration,
                };
                if !should_continue {
                    break;
                }
                attempted += 1;
                let request_started = Instant::now();
                let result = match connection.as_mut() {
                    Some(value) => value.request(&path),
                    None => Err(anyhow!("connection unavailable")),
                };
                match result {
                    Ok(body) if body == expected => {
                        latencies.push(request_started.elapsed().as_secs_f64() * 1_000.0);
                    }
                    _ => {
                        errors += 1;
                        connection = HttpConnection::connect(address, timeout_ms).ok();
                    }
                }
            }
            (latencies, errors)
        }));
    }
    barrier.wait();
    let server_ticks_before = process_tree_cpu_ticks(server_pid);
    let client_ticks_before = process_cpu_ticks_optional(std::process::id());
    let started = Instant::now();
    let mut latency_ms = Vec::new();
    let mut errors = 0;
    for handle in handles {
        let (mut values, worker_errors) =
            handle.join().map_err(|_| anyhow!("HTTP worker panicked"))?;
        latency_ms.append(&mut values);
        errors += worker_errors;
    }
    let duration_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let server_cpu_core_pct =
        cpu_core_pct(server_ticks_before, process_tree_cpu_ticks(server_pid), duration_ms);
    let client_cpu_core_pct = cpu_core_pct(
        client_ticks_before,
        process_cpu_ticks_optional(std::process::id()),
        duration_ms,
    );
    sampling.store(false, Ordering::Relaxed);
    let peak_memory = memory_sampler.join().map_err(|_| anyhow!("memory sampler panicked"))?;
    let requests = latency_ms.len().saturating_add(errors);
    let completed = requests.saturating_sub(errors);
    let requests_per_second =
        if duration_ms > 0.0 { completed as f64 / (duration_ms / 1_000.0) } else { 0.0 };
    Ok(HttpRound {
        duration_ms,
        requests,
        requests_per_second,
        latency_ms,
        errors,
        server_cpu_core_pct,
        client_cpu_core_pct,
        peak_memory_kb: peak_memory.as_ref().map(|sample| sample.value_kb),
        memory_kind: peak_memory.map(|sample| sample.kind),
    })
}

#[derive(Clone, Copy)]
enum RoundLimit {
    Requests(usize),
    Duration(Duration),
}

fn cpu_core_pct(before: Option<u64>, after: Option<u64>, duration_ms: f64) -> Option<f64> {
    let ticks = after?.checked_sub(before?)? as f64;
    let ticks_per_second = *clock_ticks_per_second()?;
    (duration_ms > 0.0).then(|| ticks / ticks_per_second / (duration_ms / 1_000.0) * 100.0)
}

fn clock_ticks_per_second() -> Option<&'static f64> {
    static TICKS: OnceLock<Option<f64>> = OnceLock::new();
    TICKS
        .get_or_init(|| {
            let output = Command::new("getconf").arg("CLK_TCK").output().ok()?;
            output.status.success().then_some(())?;
            String::from_utf8(output.stdout).ok()?.trim().parse().ok()
        })
        .as_ref()
}

fn process_tree_cpu_ticks(root_pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let pids = process_tree_pids(root_pid).ok()?;
        Some(
            pids.into_iter()
                .filter_map(|pid| process_cpu_ticks(pid).ok())
                .fold(0_u64, u64::saturating_add),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root_pid;
        None
    }
}

fn process_cpu_ticks_optional(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        process_cpu_ticks(pid).ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "linux")]
fn process_cpu_ticks(pid: u32) -> Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    parse_process_cpu_ticks(&stat)
}

#[cfg(any(target_os = "linux", test))]
fn parse_process_cpu_ticks(stat: &str) -> Result<u64> {
    let command_end = stat.rfind(')').context("invalid /proc stat command field")?;
    let fields: Vec<_> = stat[command_end + 1..].split_whitespace().collect();
    let user_ticks: u64 = fields.get(11).context("missing /proc stat utime")?.parse()?;
    let system_ticks: u64 = fields.get(12).context("missing /proc stat stime")?.parse()?;
    Ok(user_ticks.saturating_add(system_ticks))
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
        let size_text = size_line.trim().split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size_text, 16)?;
        if size == 0 {
            loop {
                let mut trailer = String::new();
                reader.read_line(&mut trailer)?;
                if trailer == "\r\n" || trailer == "\n" {
                    break;
                }
            }
            return Ok(body);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        let mut crlf = [0_u8; 2];
        reader.read_exact(&mut crlf)?;
        ensure!(crlf == *b"\r\n", "invalid chunk terminator");
    }
}

fn rotate_order(runtimes: &mut [RuntimeSpec], seed: u64) {
    if !runtimes.is_empty() {
        let length = runtimes.len();
        runtimes.rotate_left((seed as usize) % length);
    }
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_owned() } else { root.join(path) }
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_parser_handles_spaces_in_process_names() {
        let stat = "42 (worker with spaces) S 1 2 3 4 5 6 7 8 9 10 123 45 0 0";
        assert_eq!(parse_process_cpu_ticks(stat).unwrap(), 168);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cpu_sampler_reads_the_current_process() {
        process_cpu_ticks(std::process::id()).unwrap();
        assert!(clock_ticks_per_second().is_some_and(|value| *value > 0.0));
    }
}
