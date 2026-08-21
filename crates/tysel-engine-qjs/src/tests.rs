use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Frame;
use hyper::{Request as HyperRequest, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tysel_cap_llm::{
    LlmAuditSink, LlmGateway, LlmGatewayConfig, LlmProvider, LlmResponse, LlmRoute, LlmUsage,
    NoopAudit, ProviderFuture, ProviderRequest, SecretResolver, SecretValue,
};
use tysel_durable::{EventKind, NewEvent, SqliteStore};
use tysel_engine::{EngineError, HttpRequest, InterruptReason, IsolateConfig, Value};
use tysel_task::TaskId;

use crate::{
    DurableSession, IncomingHttp, IsolateCancel, IsolatePool, STREAM_WINDOW, encode_durable_export,
    eval, eval_cancellable, eval_durable, eval_durable_module, inspect_durable_exports,
};

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
fn javascript_exceptions_preserve_message_and_stack() {
    let error = eval(
        r#"(async function namedFailure() { throw new Error("preserved failure"); })()"#,
        config(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("preserved failure"), "{error}");
    assert!(error.contains("namedFailure"), "{error}");
}

struct TestLlmProvider;

impl LlmProvider for TestLlmProvider {
    fn generate<'a>(&'a self, request: ProviderRequest) -> ProviderFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.credential.as_ref().map(SecretValue::expose), Some("test-key"));
            Ok(LlmResponse {
                output: serde_json::json!({ "echo": request.request.input }),
                usage: LlmUsage { input_tokens: 2, output_tokens: 3 },
                provider_request_id: Some("test-provider-1".into()),
            })
        })
    }
}

struct TestLlmSecrets;

impl SecretResolver for TestLlmSecrets {
    fn resolve(&self, handle: &str) -> Option<SecretValue> {
        (handle == "secret:LLM_TEST_KEY").then(|| SecretValue::new("test-key").unwrap())
    }
}

#[test]
fn llm_generate_runs_through_the_native_gateway() {
    let audit: Arc<dyn LlmAuditSink> = Arc::new(NoopAudit);
    let gateway = LlmGateway::new(
        BTreeMap::from([(
            "default".into(),
            LlmRoute {
                provider_name: "test".into(),
                provider: Arc::new(TestLlmProvider),
                credential_handle: Some("secret:LLM_TEST_KEY".into()),
            },
        )]),
        Arc::new(TestLlmSecrets),
        audit,
        LlmGatewayConfig::default(),
    )
    .unwrap();
    crate::configure_llm(Some(Arc::new(gateway)));

    let value = eval(
        r#"
        (async () => {
          const response = await tysel.llm.generate({
            model: "default",
            input: { customer: 7 },
            maxOutputTokens: 20,
          });
          return JSON.stringify(response);
        })()
        "#,
        config(),
    )
    .expect("LLM generate");
    assert_eq!(
        value,
        Value::String(
            r#"{"output":{"echo":{"customer":7}},"usage":{"input_tokens":2,"output_tokens":3},"provider_request_id":"test-provider-1"}"#
                .into()
        )
    );
    crate::configure_llm(None);
}

#[test]
fn durable_step_replays_without_running_the_callback() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(101);
    let script = r#"
        (async () => {
            let calls = 0;
            const value = await tysel.durable.step("load", () => {
                calls += 1;
                return { answer: 42 };
            });
            return JSON.stringify({ value, calls });
        })()
    "#;
    let first = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("first durable run");
    assert_eq!(first, Value::String(r#"{"value":{"answer":42},"calls":1}"#.into()));
    let replayed = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("replayed durable run");
    assert_eq!(replayed, Value::String(r#"{"value":{"answer":42},"calls":0}"#.into()));
    let history = store.load_history(id).unwrap();
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].kind, EventKind::Step);
}

#[test]
fn durable_module_receives_context_and_json_input() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let value = eval_durable_module(
        r#"
        export default async function task(ctx, input) {
            const recorded = await ctx.step("input", () => input.value);
            return { recorded, attempt: input.attempt };
        }
        "#,
        r#"{"value":"hello","attempt":2}"#,
        config(),
        DurableSession::new(store, TaskId(116)).unwrap(),
    )
    .expect("durable module");
    assert_eq!(
        value,
        Value::Record(vec![
            ("recorded".into(), Value::String("hello".into())),
            ("attempt".into(), Value::Number(2.0)),
        ])
    );
}

#[test]
fn durable_named_export_resolves_from_app_table() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let source = encode_durable_export(
        "agent",
        r#"
        export default {
            durable: {
                async agent(ctx, input) {
                    const recorded = await ctx.step("input", () => input.value);
                    return { recorded };
                }
            }
        };
        "#,
    );
    let names = inspect_durable_exports(
        r#"
        export default {
            durable: {
                async agent(ctx, input) { return input; }
            }
        };
        "#,
        config(),
    )
    .expect("inspect durable");
    assert_eq!(names, vec!["agent".to_string()]);
    let value = eval_durable_module(
        &source,
        r#"{"value":"named"}"#,
        config(),
        DurableSession::new(store, TaskId(118)).unwrap(),
    )
    .expect("named durable");
    assert_eq!(value, Value::Record(vec![("recorded".into(), Value::String("named".into()))]));
}

#[test]
fn durable_module_suspends_and_resumes_from_history() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(117);
    let source = r#"
        export default async function task(ctx, input) {
            const recorded = await ctx.step("name", () => input.name);
            await ctx.sleep("30ms");
            return recorded;
        }
    "#;
    let first = eval_durable_module(
        source,
        r#"{"name":"Ada"}"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect_err("module suspends");
    assert!(matches!(first, EngineError::Suspended));

    let wakeup = store.wakeup(id).unwrap().unwrap();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    thread::sleep(Duration::from_millis(wakeup.wake_at_ms.saturating_sub(now_ms) + 1));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let claim = store.claim_due_wakeups(now_ms, 1, "module-runner", 5_000).unwrap().pop().unwrap();
    let resumed = eval_durable_module(
        source,
        r#"{"name":"changed"}"#,
        config(),
        DurableSession::from_claim(store.clone(), claim).unwrap(),
    )
    .expect("module resumes");
    assert_eq!(resumed, Value::String("Ada".into()));
    assert_eq!(store.wakeup(id).unwrap(), None);
}

#[test]
fn durable_module_records_input_before_top_level_suspension() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(118);
    let source = r#"
        await tysel.durable.waitForSignal("ready");
        export default async function task(_ctx, input) {
            return input;
        }
    "#;
    let first = eval_durable_module(
        source,
        r#"{"name":"Ada"}"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect_err("module suspends at top level");
    assert!(matches!(first, EngineError::Suspended));
    let history = store.load_history(id).unwrap();
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].key, "$tysel:task-input");

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    store.send_signal(id, "ready", &serde_json::json!(true), now_ms).unwrap();
    let claim = store.claim_due_wakeups(now_ms, 1, "module-runner", 5_000).unwrap().pop().unwrap();
    let resumed = eval_durable_module(
        source,
        r#"{"name":"changed"}"#,
        config(),
        DurableSession::from_claim(store, claim).unwrap(),
    )
    .expect("module resumes");
    assert_eq!(resumed, Value::Record(vec![("name".into(), Value::String("Ada".into()))]));
}

#[test]
fn durable_now_and_random_are_stable_on_replay() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(102);
    let script = "JSON.stringify([tysel.durable.now().toISOString(), tysel.durable.random()])";
    let first = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("first durable run");
    let replayed = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("replayed durable run");
    assert_eq!(replayed, first);
    assert_eq!(store.load_history(id).unwrap().events.len(), 2);
}

#[test]
fn durable_float_payload_keeps_its_original_json_representation() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(111);
    let script = r#"
        (async () => JSON.stringify(await tysel.durable.step(
            "float",
            () => 0.43128896623398716,
        )))()
    "#;
    let first = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("first float execution");
    let replayed = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("float replay");
    assert_eq!(replayed, first);
    assert_eq!(store.load_history(id).unwrap().events[0].payload_json(), "0.43128896623398716");
}

#[test]
fn durable_replay_rejects_changed_boundary_order() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(103);
    eval_durable(
        r#"(async () => tysel.durable.step("one", () => 1))()"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect("first durable run");
    let err = eval_durable(
        r#"(async () => tysel.durable.step("two", () => 2))()"#,
        config(),
        DurableSession::new(store, id).unwrap(),
    )
    .expect_err("changed history must be rejected");
    assert!(matches!(err, EngineError::Isolate(_)));
}

#[test]
fn durable_replay_rejects_unconsumed_history() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(104);
    eval_durable(
        r#"(async () => {
            await tysel.durable.step("one", () => 1);
            return tysel.durable.step("two", () => 2);
        })()"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect("first durable run");
    let err = eval_durable(
        r#"(async () => tysel.durable.step("one", () => 1))()"#,
        config(),
        DurableSession::new(store, id).unwrap(),
    )
    .expect_err("truncated history must be rejected");
    match err {
        EngineError::Isolate(message) => assert!(message.contains("sequence 1"), "{message}"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn durable_stale_session_cannot_append_a_second_history() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(106);
    let first_session = DurableSession::new(store.clone(), id).unwrap();
    let stale_session = DurableSession::new(store.clone(), id).unwrap();
    eval_durable(
        r#"(async () => tysel.durable.step("winner", () => 1))()"#,
        config(),
        first_session,
    )
    .expect("winning execution");
    let err = eval_durable(
        r#"(async () => tysel.durable.step("stale", () => 2))()"#,
        config(),
        stale_session,
    )
    .expect_err("stale execution must conflict");
    assert!(matches!(err, EngineError::Isolate(_)));
    let history = store.load_history(id).unwrap();
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].key, "winner");
}

#[test]
fn durable_sleep_suspends_and_clears_its_wakeup_on_replay() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(105);
    let script = r#"(async () => { await tysel.durable.sleep("50ms"); return "awake"; })()"#;
    let err = eval_durable(
        script,
        IsolateConfig { request_timeout_ms: 10, ..config() },
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect_err("first run suspends at the durable boundary");
    assert!(matches!(err, EngineError::Suspended));
    assert_eq!(store.load_history(id).unwrap().events[0].kind, EventKind::Sleep);
    let wakeup = store.wakeup(id).unwrap().expect("persisted wakeup");
    let early = match DurableSession::new(store.clone(), id) {
        Ok(_) => panic!("unclaimed task resumed"),
        Err(error) => error,
    };
    assert!(early.contains("suspended until"), "{early}");

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    thread::sleep(Duration::from_millis(wakeup.wake_at_ms.saturating_sub(now_ms) + 1));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let claim =
        store.claim_due_wakeups(now_ms, 1, "test-runner", 5_000).unwrap().pop().expect("due claim");
    assert!(store.claim_due_wakeups(now_ms, 1, "other-runner", 5_000).unwrap().is_empty());
    let replayed =
        eval_durable(script, config(), DurableSession::from_claim(store.clone(), claim).unwrap())
            .expect("claimed task resumes from recorded sleep");
    assert_eq!(replayed, Value::String("awake".into()));
    assert_eq!(store.wakeup(id).unwrap(), None);
}

#[test]
fn durable_claim_cannot_resume_before_the_real_wakeup_time() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(107);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let wake_at_ms = now_ms + 60_000;
    store
        .append_event_with_wakeup(
            id,
            NewEvent {
                kind: EventKind::Sleep,
                key: "sleep:future".into(),
                payload: serde_json::json!({"duration_ms": 60_000}),
                recorded_at_ms: now_ms,
            },
            wake_at_ms,
        )
        .unwrap();
    let claim = store
        .claim_due_wakeups(wake_at_ms, 1, "bad-clock-runner", 5_000)
        .unwrap()
        .pop()
        .expect("claim made with an invalid future clock");
    let error = match DurableSession::from_claim(store, claim) {
        Ok(_) => panic!("future wakeup resumed"),
        Err(error) => error,
    };
    assert!(error.contains("not due until"), "{error}");
}

#[test]
fn durable_signal_sent_before_wait_is_replayed() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(108);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    store.send_signal(id, "approval", &serde_json::json!({"approved": true}), now_ms).unwrap();
    let script = r#"
        (async () => JSON.stringify(await tysel.durable.waitForSignal("approval")))()
    "#;
    let first = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("queued signal is consumed");
    let replayed = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("signal result replays from history");
    assert_eq!(first, Value::String(r#"{"approved":true}"#.into()));
    assert_eq!(replayed, first);
    let history = store.load_history(id).unwrap();
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].kind, EventKind::Signal);
}

#[test]
fn durable_signal_wakes_a_suspended_task_through_a_claim() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(109);
    let script = r#"
        (async () => JSON.stringify(await tysel.durable.waitForSignal("approval")))()
    "#;
    let started = Instant::now();
    let err = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect_err("task suspends while the signal is absent");
    assert!(matches!(err, EngineError::Suspended));
    assert!(started.elapsed() < Duration::from_secs(1), "suspension waited for the deadline");
    assert!(store.load_history(id).unwrap().events.is_empty());
    assert_eq!(store.signal_wait(id).unwrap().unwrap().signal_name, "approval");
    let suspended = match DurableSession::new(store.clone(), id) {
        Ok(_) => panic!("signal wait resumed without a wakeup claim"),
        Err(error) => error,
    };
    assert!(suspended.contains("waiting for signal"), "{suspended}");

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    store.send_signal(id, "approval", &serde_json::json!({"approved": true}), now_ms).unwrap();
    let claim = store
        .claim_due_wakeups(now_ms, 1, "signal-runner", 5_000)
        .unwrap()
        .pop()
        .expect("signal wakeup claim");
    assert!(store.claim_due_wakeups(now_ms, 1, "other-runner", 5_000).unwrap().is_empty());
    let resumed =
        eval_durable(script, config(), DurableSession::from_claim(store.clone(), claim).unwrap())
            .expect("claimed signal task resumes");
    assert_eq!(resumed, Value::String(r#"{"approved":true}"#.into()));
    assert_eq!(store.signal_wait(id).unwrap(), None);
    assert_eq!(store.wakeup(id).unwrap(), None);
    assert_eq!(store.load_history(id).unwrap().events[0].kind, EventKind::Signal);
}

#[test]
fn durable_signal_wait_cannot_be_started_and_ignored() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(110);
    let error = eval_durable(
        r#"(() => { tysel.durable.waitForSignal("approval"); return 42; })()"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect_err("unawaited signal wait must not complete the task");
    assert!(matches!(
        error,
        EngineError::Isolate(message) if message.contains("persisted suspension")
    ));
    assert!(store.signal_wait(id).unwrap().is_some());
}

#[test]
fn durable_retry_suspends_for_backoff_and_resumes_at_the_next_attempt() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(112);
    let script = r#"
        (async () => tysel.durable.retry(
            { maxAttempts: 3, delay: "30ms", factor: 2 },
            async (attempt) => {
                await tysel.durable.step("attempt-" + attempt, () => attempt);
                if (attempt < 2) throw new TypeError("transient");
                return attempt;
            },
        ))()
    "#;
    let error = eval_durable(script, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect_err("first retry attempt suspends for backoff");
    assert!(matches!(error, EngineError::Suspended));
    let history = store.load_history(id).unwrap();
    assert_eq!(
        history.events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        vec![EventKind::Retry, EventKind::Step, EventKind::Retry, EventKind::Sleep]
    );

    let wakeup = store.wakeup(id).unwrap().unwrap();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    thread::sleep(Duration::from_millis(wakeup.wake_at_ms.saturating_sub(now_ms) + 1));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let claim = store.claim_due_wakeups(now_ms, 1, "retry-runner", 5_000).unwrap().pop().unwrap();
    let resumed =
        eval_durable(script, config(), DurableSession::from_claim(store.clone(), claim).unwrap())
            .expect("retry resumes at its second attempt");
    assert_eq!(resumed, Value::Number(2.0));
    let history = store.load_history(id).unwrap();
    assert_eq!(history.events.iter().filter(|event| event.key == "attempt-1").count(), 1);
    assert!(history.events.last().unwrap().key.ends_with(":outcome:2"));
    assert_eq!(store.wakeup(id).unwrap(), None);
}

#[test]
fn durable_retry_replays_a_success_without_rerunning_its_callback() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(115);
    let first = r#"
        (async () => {
            const value = await tysel.durable.retry(
                { maxAttempts: 1 },
                () => "recorded-success",
            );
            await tysel.durable.sleep("30ms");
            return value;
        })()
    "#;
    let changed = r#"
        (async () => {
            const value = await tysel.durable.retry(
                { maxAttempts: 1 },
                () => { throw new Error("must not rerun"); },
            );
            await tysel.durable.sleep("30ms");
            return value;
        })()
    "#;
    let error = eval_durable(first, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect_err("later sleep suspends the task");
    assert!(matches!(error, EngineError::Suspended));

    let wakeup = store.wakeup(id).unwrap().unwrap();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    thread::sleep(Duration::from_millis(wakeup.wake_at_ms.saturating_sub(now_ms) + 1));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let claim = store.claim_due_wakeups(now_ms, 1, "retry-success", 5_000).unwrap().pop().unwrap();
    let replayed =
        eval_durable(changed, config(), DurableSession::from_claim(store.clone(), claim).unwrap())
            .expect("recorded success skips the changed callback");
    assert_eq!(replayed, Value::String("recorded-success".into()));
    assert_eq!(store.load_history(id).unwrap().events.len(), 3);
}

#[test]
fn durable_retry_replays_recorded_failures_when_the_callback_changes() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(113);
    let failing = r#"
        (async () => {
            try {
                return await tysel.durable.retry(
                    { maxAttempts: 2 },
                    (attempt) => { throw new TypeError("nope-" + attempt); },
                );
            } catch (error) {
                return error.name + ":" + error.message;
            }
        })()
    "#;
    let changed = r#"
        (async () => {
            try {
                return await tysel.durable.retry(
                    { maxAttempts: 2 },
                    () => "unexpected-success",
                );
            } catch (error) {
                return error.name + ":" + error.message;
            }
        })()
    "#;
    let first = eval_durable(failing, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("retry failure is caught");
    let replayed = eval_durable(changed, config(), DurableSession::new(store.clone(), id).unwrap())
        .expect("recorded failures override changed callback outcomes");
    assert_eq!(first, Value::String("TypeError:nope-2".into()));
    assert_eq!(replayed, first);
    assert_eq!(store.load_history(id).unwrap().events.len(), 4);
}

#[test]
fn durable_retry_rejects_invalid_policy_without_writing_history() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let id = TaskId(114);
    let error = eval_durable(
        r#"(async () => tysel.durable.retry({ maxAttempts: 0 }, () => "never"))()"#,
        config(),
        DurableSession::new(store.clone(), id).unwrap(),
    )
    .expect_err("invalid retry policy");
    assert!(matches!(error, EngineError::Isolate(_)));
    assert!(store.load_history(id).unwrap().events.is_empty());
    assert_eq!(store.wakeup(id).unwrap(), None);
}

#[test]
fn secret_ref_returns_opaque_handle() {
    let value = eval(r#"(async () => tysel.secrets.ref("db"))()"#, config()).expect("eval");
    assert_eq!(value, Value::String("secret:db".into()));
}

#[test]
fn sqlite_query_roundtrips_bound_params() {
    let value = eval(
        r#"
        (async () => {
            const t = "t_" + Math.random().toString(16).slice(2);
            await tysel.sqlite.exec("CREATE TABLE " + t + " (id INTEGER, name TEXT)");
            const changes = await tysel.sqlite.exec(
                "INSERT INTO " + t + " (id, name) VALUES (?, ?)",
                [1, "o'reilly"]
            );
            if (changes !== 1) return "changes=" + changes;
            const rows = await tysel.sqlite.query(
                "SELECT id, name FROM " + t + " WHERE name = ?",
                ["o'reilly"]
            );
            return JSON.stringify(rows);
        })()
        "#,
        config(),
    )
    .expect("eval");
    assert_eq!(value, Value::String(r#"[{"id":1,"name":"o'reilly"}]"#.into()));
}

#[test]
fn postgres_is_denied_until_configured() {
    let value = eval(
        r#"
        (async () => {
            try {
                await tysel.postgres.query("SELECT 1");
                return "allowed";
            } catch (err) {
                return String(err);
            }
        })()
        "#,
        config(),
    )
    .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("not configured"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn filesystem_is_denied_until_configured() {
    let value = eval(
        r#"
        (async () => {
            try {
                await tysel.fs.read("hello.txt");
                return "allowed";
            } catch (err) {
                return String(err);
            }
        })()
        "#,
        config(),
    )
    .expect("eval");
    match value {
        Value::String(message) => {
            assert!(message.contains("not configured"), "unexpected error: {message}");
        }
        other => panic!("expected error string, got {other:?}"),
    }
}

#[test]
fn sqlite_timeout_keeps_the_connection_usable() {
    let started = Instant::now();
    let result = eval(
        r#"(async () => tysel.sqlite.query(
            "WITH RECURSIVE t(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM t WHERE x < 200000000)
             SELECT COUNT(*) AS n FROM t"
        ))()"#,
        IsolateConfig { request_timeout_ms: 80, ..config() },
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "sqlite query was not interrupted ({:?})",
        started.elapsed()
    );
    match result {
        Err(EngineError::Interrupted(InterruptReason::Timeout)) => {}
        Err(EngineError::Isolate(_)) => {}
        Ok(Value::String(message))
            if message.to_ascii_lowercase().contains("timeout")
                || message.to_ascii_lowercase().contains("interrupt") => {}
        other => panic!("unexpected result: {other:?}"),
    }
    let value = eval(
        r#"
        (async () => {
            const t = "t_after_" + Math.random().toString(16).slice(2);
            await tysel.sqlite.exec("CREATE TABLE " + t + " (id INTEGER)");
            await tysel.sqlite.exec("INSERT INTO " + t + " (id) VALUES (7)");
            const rows = await tysel.sqlite.query("SELECT id FROM " + t);
            return JSON.stringify(rows);
        })()
        "#,
        config(),
    )
    .expect("sqlite after timeout");
    assert_eq!(value, Value::String(r#"[{"id":7}]"#.into()));
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
fn crypto_subtle_digest_and_hmac_match_known_vectors() {
    let value = eval(
        r#"(async () => {
            const empty = new Uint8Array();
            const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", empty));
            const hex = Array.from(digest).map((b) => b.toString(16).padStart(2, "0")).join("");
            const key = await crypto.subtle.importKey(
                "raw",
                new TextEncoder().encode("key"),
                { name: "HMAC", hash: "SHA-256" },
                false,
                ["sign", "verify"],
            );
            const data = new TextEncoder().encode("The quick brown fox jumps over the lazy dog");
            const signature = new Uint8Array(await crypto.subtle.sign("HMAC", key, data));
            const hmac = Array.from(signature).map((b) => b.toString(16).padStart(2, "0")).join("");
            const verified = await crypto.subtle.verify("HMAC", key, signature, data);
            return JSON.stringify({ hex, hmac, verified });
        })()"#,
        config(),
    )
    .expect("eval");
    match value {
        Value::String(json) => {
            assert!(
                json.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            );
            assert!(
                json.contains("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8")
            );
            assert!(json.contains("\"verified\":true"));
        }
        other => panic!("expected JSON string, got {other:?}"),
    }
}

#[test]
fn crypto_subtle_enforces_hmac_key_usages() {
    let value = eval(
        r#"(async () => {
          const data = new TextEncoder().encode("payload");
          const raw = new TextEncoder().encode("secret");
          const signing = await crypto.subtle.importKey("raw", raw, { name: "hmac", hash: "sha256" }, false, ["sign"]);
          const signature = new Uint8Array(await crypto.subtle.sign("hmac", signing, data));
          signature[0] ^= 1;
          const verifying = await crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
          const altered = await crypto.subtle.verify("HMAC", verifying, signature, data);
          let denied = "";
          try { await crypto.subtle.sign("HMAC", verifying, data); } catch (error) { denied = error.name; }
          let invalid = "";
          try { await crypto.subtle.importKey("raw", raw, "HMAC", false, ["encrypt"]); } catch (error) { invalid = error.name; }
          return JSON.stringify({ altered, denied, invalid, cryptoKey: signing instanceof CryptoKey });
        })()"#,
        config(),
    )
    .expect("web crypto key semantics");
    assert_eq!(
        value,
        Value::String(
            r#"{"altered":false,"denied":"InvalidAccessError","invalid":"SyntaxError","cryptoKey":true}"#.into()
        )
    );
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
            request_id: 0,
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
            request_id: 0,
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

const SQLITE_HANDLER: &str = r#"
export default {
  async fetch() {
    await tysel.sqlite.exec(
      "CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value INTEGER NOT NULL)"
    );
    await tysel.sqlite.exec(
      "INSERT INTO kv(key, value) VALUES ('hits', 1) ON CONFLICT(key) DO UPDATE SET value = value + 1"
    );
    const rows = await tysel.sqlite.query("SELECT value FROM kv WHERE key = ?", ["hits"]);
    return Response.json({ value: rows[0].value });
  },
};
"#;

#[tokio::test]
async fn fetch_handler_sqlite_increments_counter() {
    let pool = IsolatePool::spawn(1, SQLITE_HANDLER, config()).expect("spawn isolate");
    let (head, mut body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch");
    assert_eq!(head.status, 200);
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    let json = String::from_utf8(bytes).expect("utf8");
    assert_eq!(json, r#"{"value":1}"#);
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
            request_id: 0,
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
        request_id: 0,
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
            request_id: 0,
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
        request_id: 0,
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
async fn fetch_exposes_response_headers() {
    let addr = serve_header("x-request-id", "abc", Bytes::from_static(b"ok"));
    let url = format!("http://{addr}/");
    let value = tokio::task::spawn_blocking(move || {
        eval(
            &format!(
                r#"(async () => {{
                    const res = await fetch("{url}");
                    return [
                        res.headers.get("x-request-id"),
                        res.headers.get("connection") || "none",
                        await res.text(),
                    ].join(":");
                }})()"#
            ),
            config(),
        )
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("abc:none:ok".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_redirect_keeps_authorization() {
    let addr = serve_auth_redirect_same_origin();
    let url = format!("http://{addr}/go");
    let value = tokio::task::spawn_blocking(move || {
        eval(
            &format!(
                r#"(async () => (await fetch("{url}", {{
                    headers: {{ Authorization: "Bearer kept" }},
                }})).text())()"#
            ),
            config(),
        )
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("Bearer kept".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_redirect_strips_authorization() {
    let dest = serve_auth_echo();
    let src = serve_redirect_to(format!("http://{dest}/"));
    let url = format!("http://{src}/");
    let value = tokio::task::spawn_blocking(move || {
        eval(
            &format!(
                r#"(async () => (await fetch("{url}", {{
                    headers: {{ Authorization: "Bearer leaked" }},
                }})).text())()"#
            ),
            config(),
        )
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("none".into()));
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
async fn fetch_posts_a_body() {
    let addr = serve_echo();
    let url = format!("http://{addr}/");
    let value = tokio::task::spawn_blocking(move || {
        eval(
            &format!(
                r#"(async () => (await fetch("{url}", {{
                    method: "POST",
                    headers: {{ "content-type": "text/plain" }},
                    body: "hello",
                }})).text())()"#
            ),
            config(),
        )
    })
    .await
    .expect("join")
    .expect("eval");
    assert_eq!(value, Value::String("POST:hello".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_rejects_unsupported_method() {
    let value = tokio::task::spawn_blocking(|| {
        eval(
            r#"(async () => {
                try {
                    await fetch("http://127.0.0.1:1/", { method: "TRACE" });
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
            assert!(
                message.contains("GET, HEAD, POST, PUT, PATCH, and DELETE"),
                "unexpected error: {message}"
            );
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
        request_id: 0,
    });
    let (head, _body) = dispatch.await.expect("dispatch");
    assert_eq!(head.status, 101);
    assert!(head.websocket);
    to_js_tx.send(Ok(b"ping".to_vec())).await.unwrap();
    let echoed = from_js_rx.recv().await.expect("echo");
    assert_eq!(echoed, b"ping");
    drop(to_js_tx);
}

#[tokio::test(flavor = "multi_thread")]
async fn outbound_websocket_echoes_text() {
    use futures_util::{SinkExt, StreamExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind websocket");
    let addr = listener.local_addr().expect("websocket address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream).await.expect("handshake");
        if let Some(Ok(message)) = socket.next().await {
            socket.send(message).await.expect("echo websocket message");
        }
        let _ = socket.close(None).await;
    });

    let source = format!(
        r#"(async () => new Promise((resolve, reject) => {{
            const socket = new WebSocket("ws://{addr}/echo");
            socket.onopen = async () => {{ await socket.send("ping"); }};
            socket.onmessage = async (event) => {{ await socket.close(); resolve(event.data); }};
            socket.onerror = (event) => reject(event.error);
        }}))()"#
    );
    let value = tokio::task::spawn_blocking(move || eval(&source, config()))
        .await
        .expect("join")
        .expect("outbound websocket");
    assert_eq!(value, Value::String("ping".into()));
}

fn serve_bytes(body: Bytes) -> SocketAddr {
    spawn_origin(move |_| {
        let body = body.clone();
        async move { Ok::<_, Infallible>(Response::new(http_body_util::Full::new(body))) }
    })
}

fn serve_header(name: &'static str, value: &'static str, body: Bytes) -> SocketAddr {
    spawn_origin(move |_| {
        let body = body.clone();
        async move {
            Ok::<_, Infallible>(
                Response::builder()
                    .header(name, value)
                    .header("connection", "close")
                    .body(http_body_util::Full::new(body))
                    .unwrap(),
            )
        }
    })
}

fn serve_echo() -> SocketAddr {
    spawn_origin(|req| async move {
        let method = req.method().as_str().to_owned();
        let collected = req.collect().await.expect("body");
        let payload = collected.to_bytes();
        let body = format!("{method}:{}", String::from_utf8_lossy(&payload));
        Ok::<_, Infallible>(Response::new(http_body_util::Full::new(Bytes::from(body))))
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

fn serve_auth_echo() -> SocketAddr {
    spawn_origin(|req| async move {
        let auth = req
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none")
            .to_owned();
        Ok::<_, Infallible>(Response::new(http_body_util::Full::new(Bytes::from(auth))))
    })
}

fn serve_auth_redirect_same_origin() -> SocketAddr {
    spawn_origin(|req| async move {
        if req.uri().path() == "/go" {
            Ok(Response::builder()
                .status(302)
                .header("location", "/done")
                .body(http_body_util::Full::new(Bytes::new()))
                .unwrap())
        } else {
            let auth = req
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .to_owned();
            Ok(Response::new(http_body_util::Full::new(Bytes::from(auth))))
        }
    })
}

fn serve_redirect_to(location: String) -> SocketAddr {
    spawn_origin(move |_| {
        let location = location.clone();
        async move {
            Ok::<_, Infallible>(
                Response::builder()
                    .status(302)
                    .header("location", location)
                    .body(http_body_util::Full::new(Bytes::new()))
                    .unwrap(),
            )
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
