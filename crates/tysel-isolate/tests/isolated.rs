use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tysel_engine::{HttpRequest, Value};
use tysel_isolate::{IsolatedHttpPool, Supervisor, WorkerSpec};

#[test]
fn eval_echo_runs_in_broker() {
    let mut supervisor = supervisor();
    let value = supervisor.eval(r#"(async () => tysel.echo("hello"))()"#).expect("eval");
    assert_eq!(value, Value::String("hello".into()));
}

#[test]
fn worker_env_does_not_inherit_supervisor_environment() {
    let mut supervisor = supervisor();
    let value = supervisor.eval("tysel.envKeys()").expect("eval");
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
fn isolated_sleep_resolves_without_broker() {
    let mut supervisor = supervisor();
    let value =
        supervisor.eval(r#"(async () => { await tysel.sleep(20); return 7; })()"#).expect("sleep");
    assert_eq!(value, Value::Number(7.0));
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
        })
        .expect("dispatch");
    assert_eq!(head.status, 200);
    assert_eq!(body, b"ok");
}

#[test]
fn isolated_http_handler_does_not_see_supervisor_env() {
    let pool = IsolatedHttpPool::spawn(
        worker_exe(),
        r#"export default { async fetch() { return new Response("ENV:" + tysel.envKeys() + ":END"); } };"#,
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
