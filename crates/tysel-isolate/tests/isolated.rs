use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use tysel_engine::{HttpRequest, Value};
use tysel_isolate::{IsolatedHttpPool, IsolatedTaskPool, Supervisor, WorkerSpec};

#[test]
fn eval_echo_runs_in_broker() {
    let mut supervisor = supervisor();
    let value = supervisor.eval(r#"(async () => tysel.echo("hello"))()"#).expect("eval");
    assert_eq!(value, Value::String("hello".into()));
}

#[test]
fn isolated_task_module_runs_and_recovers_in_worker() {
    let source = r#"
export default {
  tasks: {
    echo: {
      kind: "queue",
      name: "events",
      async handler(input) { return { value: await tysel.echo(input.value) }; }
    }
  }
};
"#;
    let config = tysel_engine::IsolateConfig {
        memory_limit_bytes: 16 * 1024 * 1024,
        cpu_ms_per_turn: 2_000,
        request_timeout_ms: 5_000,
    };
    let (pool, definitions) =
        IsolatedTaskPool::spawn_from_config(worker_exe(), source, config, Vec::new())
            .expect("spawn isolated task worker");
    assert_eq!(definitions.len(), 1);
    let deadline =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
            as u64
            + 5_000;
    let value = pool
        .invoke_sync("echo", r#"{"value":"ready"}"#, "task-1", deadline)
        .expect("invoke isolated task");
    assert_eq!(value, Value::Record(vec![("value".into(), Value::String("ready".into()))]));

    pool.kill_worker().expect("kill task worker");
    let value = pool
        .invoke_sync("echo", r#"{"value":"again"}"#, "task-2", deadline)
        .expect("invoke after worker restart");
    assert_eq!(value, Value::Record(vec![("value".into(), Value::String("again".into()))]));
}

#[test]
fn worker_env_does_not_inherit_supervisor_environment() {
    let mut supervisor = supervisor();
    let value = supervisor.eval("tysel._envKeys()").expect("eval");
    let Value::String(keys) = value else {
        panic!("expected env key string, got {value:?}");
    };
    for leaked in ["HOME", "USER", "PATH", "TYSEL_TEST_SECRET"] {
        assert!(!keys.split(',').any(|key| key == leaked), "worker inherited {leaked}: {keys}");
    }
}

#[test]
fn secret_ref_returns_handle_not_raw_secret() {
    let mut supervisor = supervisor();
    let value = supervisor.eval(r#"(async () => tysel.secrets.ref("db"))()"#).expect("eval");
    assert_eq!(value, Value::String("secret:db".into()));
}

#[test]
fn unknown_secret_is_rejected_in_isolated_worker() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                try {
                    await tysel.secrets.ref("missing");
                    return "allowed";
                } catch (err) {
                    return String(err);
                }
            })()"#,
        )
        .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("unknown secret missing"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn kill_worker_recovers_on_next_eval() {
    let mut supervisor = supervisor();
    supervisor.kill_worker().expect("kill");
    let value = supervisor.eval("1 + 1").expect("eval after crash");
    assert_eq!(value, Value::Number(2.0));
}

#[test]
fn kill_worker_recovers_on_next_http() {
    let mut supervisor = Supervisor::spawn(worker_exe(), spec(), HashMap::new()).expect("spawn");
    supervisor
        .load_handler(
            r#"export default { async fetch() { return new Response("ok"); } };"#,
            Vec::new(),
        )
        .expect("load");
    let request = HttpRequest {
        method: "GET".into(),
        url: "http://tysel.local/".into(),
        headers: Vec::new(),
        body: Vec::new(),
        request_id: 0,
    };
    let (head, body) = supervisor.http(&request).expect("http");
    assert_eq!(head.status, 200);
    assert_eq!(body, b"ok");
    let worker = supervisor.worker_pid().expect("worker pid");
    // Exercise the real race: IPC can observe EOF before try_wait reports exit.
    assert!(
        Command::new("kill")
            .args(["-KILL", &worker.to_string()])
            .status()
            .expect("kill worker")
            .success()
    );
    // If EOF races process exit detection, the first request has an uncertain
    // outcome and must fail without transparent replay. A new request recovers.
    let response = supervisor.http(&request);
    let (head, body) = response.or_else(|_| supervisor.http(&request)).expect("http after kill");
    assert_eq!(head.status, 200);
    assert_eq!(body, b"ok");
}

#[test]
fn isolated_sleep_resolves_without_broker() {
    let mut supervisor = supervisor();
    let value =
        supervisor.eval(r#"(async () => { await tysel.sleep(20); return 7; })()"#).expect("sleep");
    assert_eq!(value, Value::Number(7.0));
}

#[test]
fn isolated_timer_fires_while_a_longer_sleep_is_pending() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                let fired = false;
                setTimeout(() => { fired = true; }, 10);
                await tysel.sleep(150);
                return fired;
            })()"#,
        )
        .expect("concurrent timer and sleep");
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn isolated_sleep_allows_broker_calls_and_operation_cancellation() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                const operation = tysel._sleepOp(60000);
                const result = operation.promise.then(() => "resolved", error => String(error));
                const echo = await tysel.echo("ready");
                tysel._cancelOp(operation.id);
                return echo === "ready" && (await result).includes("Cancelled");
            })()"#,
        )
        .expect("broker call and canceled sleep");
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn isolated_cleared_timers_release_io_capacity() {
    let mut supervisor = supervisor();
    let worker = supervisor.worker_pid();
    let value = supervisor
        .eval(
            r#"(async () => {
                let fired = 0;
                const pendingCount = () => Object.keys(globalThis.__tysel_pending).length;
                // More than 256 operations in one eval, recycled in bounded batches.
                for (let round = 0; round < 10; round++) {
                    const timers = Array.from({length: 32}, () => setTimeout(() => fired++, 60000));
                    const interval = setInterval(() => fired++, 60000);
                    await tysel.sleep(10);
                    timers.forEach(clearTimeout);
                    clearInterval(interval);
                    for (let retry = 0; retry < 100 && pendingCount() > 0; retry++) {
                        await tysel.sleep(5);
                    }
                    if (pendingCount() !== 0) throw new Error("canceled timers retained I/O slots");
                }
                return fired;
            })()"#,
        )
        .expect("clear timers and reuse I/O capacity");
    assert_eq!(value, Value::Number(0.0));
    assert_eq!(supervisor.eval("tysel.echo('next')").unwrap(), Value::String("next".into()));
    assert_eq!(supervisor.worker_pid(), worker, "successful evals must reuse the worker");
}

#[test]
fn sqlite_is_denied_in_isolated_worker() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                try {
                    await tysel.sqlite.exec("SELECT 1");
                    return "allowed";
                } catch (err) {
                    return String(err);
                }
            })()"#,
        )
        .expect("eval");
    match value {
        Value::String(message) => {
            assert!(
                message.contains("capability is not available in the isolated worker"),
                "unexpected error: {message}"
            );
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn postgres_is_denied_in_isolated_worker() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                try {
                    await tysel.postgres.query("SELECT 1");
                    return "allowed";
                } catch (err) {
                    return String(err);
                }
            })()"#,
        )
        .expect("eval");
    match value {
        Value::String(message) => {
            assert!(
                message.contains("capability is not available in the isolated worker"),
                "unexpected error: {message}"
            );
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn filesystem_is_denied_in_isolated_worker() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                try {
                    await tysel.fs.read("hello.txt");
                    return "allowed";
                } catch (err) {
                    return String(err);
                }
            })()"#,
        )
        .expect("eval");
    match value {
        Value::String(message) => {
            assert!(
                message.contains("capability is not available in the isolated worker"),
                "unexpected error: {message}"
            );
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn fetch_is_denied_in_isolated_worker() {
    let mut supervisor = supervisor();
    let value = supervisor
        .eval(
            r#"(async () => {
                try {
                    await fetch("http://127.0.0.1/");
                    return "allowed";
                } catch (err) {
                    return String(err);
                }
            })()"#,
        )
        .expect("eval");
    match value {
        Value::String(message) => {
            assert!(
                message.contains("capability is not available in the isolated worker"),
                "unexpected error: {message}"
            );
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn sleep_timeout_keeps_supervisor_live() {
    let mut supervisor =
        Supervisor::spawn(worker_exe(), WorkerSpec { request_timeout_ms: 80, ..spec() }, secrets())
            .expect("spawn");
    let started = Instant::now();
    let err = supervisor.eval("(async () => tysel.sleep(5000))()").expect_err("timeout");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "supervisor stayed blocked for {:?}",
        started.elapsed()
    );
    assert!(
        err.to_string().to_ascii_lowercase().contains("timeout")
            || err.to_string().contains("Interrupted"),
        "error was {err}"
    );
    let value = supervisor.eval("1 + 1").expect("eval after timeout");
    assert_eq!(value, Value::Number(2.0));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_overalloc_kills_worker_and_recovers() {
    let mut supervisor = Supervisor::spawn(worker_exe(), spec(), secrets()).expect("spawn");
    supervisor.overalloc().expect("worker should die under RLIMIT_AS");
    let value = supervisor.eval("1 + 1").expect("eval after overalloc");
    assert_eq!(value, Value::Number(2.0));
}

#[test]
fn isolated_http_handler_runs_in_the_worker() {
    let pool = IsolatedHttpPool::spawn(
        worker_exe(),
        r#"export default { async fetch() { return new Response("ok"); } };"#,
        spec(),
        Vec::new(),
    )
    .expect("spawn isolated http");
    let (head, body) = pool
        .dispatch_sync(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            request_id: 0,
        })
        .expect("dispatch");
    assert_eq!(head.status, 200);
    assert_eq!(body, b"ok");
}

#[test]
fn isolated_http_handler_does_not_see_supervisor_env() {
    let pool = IsolatedHttpPool::spawn(
        worker_exe(),
        r#"export default { async fetch() { return new Response("ENV:" + tysel._envKeys() + ":END"); } };"#,
        spec(),
        Vec::new(),
    )
    .expect("spawn isolated http");
    let (_head, body) = pool
        .dispatch_sync(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            request_id: 0,
        })
        .expect("dispatch");
    let text = String::from_utf8(body).expect("utf8");
    let keys =
        text.strip_prefix("ENV:").and_then(|rest| rest.strip_suffix(":END")).unwrap_or(&text);
    for leaked in ["HOME", "USER", "PATH", "TYSEL_TEST_SECRET"] {
        assert!(!keys.split(',').any(|key| key == leaked), "worker inherited {leaked}: {keys}");
    }
}

#[test]
fn isolated_http_denies_outbound_fetch() {
    let pool = IsolatedHttpPool::spawn(
        worker_exe(),
        r#"export default {
          async fetch() {
            try {
              await fetch("http://127.0.0.1/");
              return new Response("allowed");
            } catch (err) {
              return new Response(String(err), { status: 403 });
            }
          },
        };"#,
        spec(),
        Vec::new(),
    )
    .expect("spawn isolated http");
    let (head, body) = pool
        .dispatch_sync(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            request_id: 0,
        })
        .expect("dispatch");
    assert_eq!(head.status, 403);
    let message = String::from_utf8_lossy(&body);
    assert!(
        message.contains("isolated profile") || message.contains("isolated worker"),
        "unexpected error: {message}"
    );
}

fn supervisor() -> Supervisor {
    Supervisor::spawn(worker_exe(), spec(), secrets()).expect("spawn worker")
}

fn spec() -> WorkerSpec {
    WorkerSpec { cpu_ms_per_turn: 2_000, request_timeout_ms: 5_000, ..WorkerSpec::default() }
}

fn secrets() -> HashMap<String, String> {
    HashMap::from([("db".into(), "super-secret-password".into())])
}

fn worker_exe() -> PathBuf {
    for key in ["CARGO_BIN_EXE_tysel_worker", "CARGO_BIN_EXE_tysel-worker"] {
        if let Some(path) = std::env::var_os(key) {
            return PathBuf::from(path);
        }
    }
    let test_exe = std::env::current_exe().expect("current_exe");
    let mut candidate = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("tysel-worker"))
        .expect("target debug directory");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    assert!(candidate.is_file(), "missing tysel-worker at {}", candidate.display());
    candidate
}

#[test]
fn isolated_http_deadline_and_cancelled_queue_do_not_execute_late_work() {
    use std::sync::atomic::AtomicBool;
    use tysel_engine::{EngineError, InterruptReason};
    use tysel_isolate::IsolateError;
    let pool = IsolatedHttpPool::spawn(
        worker_exe(),
        r#"
        let count = 0;
        export default {async fetch(req) {
            const path = new URL(req.url).pathname;
            if (path === '/hang') await new Promise(() => {});
            if (path === '/mutate') count++;
            return new Response(String(count));
        }};
    "#,
        spec(),
        Vec::new(),
    )
    .unwrap();
    let request = |path: &str| HttpRequest {
        method: "GET".into(),
        url: format!("http://local{path}"),
        ..Default::default()
    };
    assert!(matches!(
        pool.dispatch_sync_until(request("/mutate"), Instant::now()),
        Err(IsolateError::Engine(EngineError::Interrupted(InterruptReason::Timeout)))
    ));
    let cancelled = std::sync::Arc::new(AtomicBool::new(true));
    assert!(matches!(
        pool.dispatch_sync_cancellable(
            request("/mutate"),
            Instant::now() + Duration::from_secs(1),
            &cancelled
        ),
        Err(IsolateError::Engine(EngineError::Interrupted(InterruptReason::Cancelled)))
    ));
    assert!(matches!(
        pool.dispatch_sync_until(request("/hang"), Instant::now() + Duration::from_millis(100)),
        Err(IsolateError::Engine(EngineError::Interrupted(InterruptReason::Timeout)))
    ));
    assert_eq!(pool.dispatch_sync(request("/count")).unwrap().1, b"0");
}

#[cfg(unix)]
#[test]
fn stopped_worker_is_killed_at_deadline_and_replaced() {
    use tysel_engine::{EngineError, InterruptReason};
    use tysel_isolate::IsolateError;
    let mut supervisor = Supervisor::spawn(worker_exe(), spec(), HashMap::new()).unwrap();
    supervisor.load_handler("export default {fetch(){return new Response('ok')}}", vec![]).unwrap();
    let pid = supervisor.worker_pid().unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let started = Instant::now();
    let request = HttpRequest { url: "http://local/".into(), ..Default::default() };
    assert!(matches!(
        supervisor.http_until(&request, started + Duration::from_millis(100)),
        Err(IsolateError::Engine(EngineError::Interrupted(InterruptReason::Timeout)))
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(supervisor.worker_pid().is_none(), "timed-out worker must be reaped before returning");
    assert_eq!(supervisor.http(&request).unwrap().1, b"ok");
    assert_ne!(supervisor.worker_pid(), Some(pid));
}

#[cfg(unix)]
#[test]
fn active_cancellation_interrupts_blocking_ipc() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tysel_engine::{EngineError, InterruptReason};
    use tysel_isolate::IsolateError;
    let mut supervisor = Supervisor::spawn(worker_exe(), spec(), HashMap::new()).unwrap();
    supervisor.load_handler("export default {fetch(){return new Response('ok')}}", vec![]).unwrap();
    let pid = supervisor.worker_pid().unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let trigger = cancelled.clone();
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        trigger.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let request = HttpRequest { url: "http://local/".into(), ..Default::default() };
    assert!(matches!(
        supervisor.http_cancellable(&request, started + Duration::from_secs(5), Some(cancelled)),
        Err(IsolateError::Engine(EngineError::Interrupted(InterruptReason::Cancelled)))
    ));
    cancel.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(supervisor.worker_pid().is_none());
}
