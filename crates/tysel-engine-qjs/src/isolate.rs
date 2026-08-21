use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Module, Promise, Runtime};
use tysel_engine::{EngineError, InterruptReason, IsolateConfig, Value};

use crate::cpu::CpuBudget;
use crate::durable::DurableSession;
use crate::host;
use crate::queue::{self, IoCompletion};

#[derive(Clone, Copy)]
enum Evaluation<'a> {
    Script(&'a str),
    DurableModule { source: &'a str, input_json: &'a str },
}

impl Evaluation<'_> {
    fn is_durable_module(self) -> bool {
        matches!(self, Self::DurableModule { .. })
    }
}

#[derive(Clone, Default)]
pub struct IsolateCancel(Arc<AtomicBool>);

impl IsolateCancel {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

pub fn eval(script: &str, config: IsolateConfig) -> Result<Value, EngineError> {
    eval_cancellable(script, config, IsolateCancel::new())
}

pub fn eval_cancellable(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
) -> Result<Value, EngineError> {
    eval_cancellable_with_durable(script, config, cancel, None)
}

pub fn eval_durable(
    script: &str,
    config: IsolateConfig,
    session: DurableSession,
) -> Result<Value, EngineError> {
    eval_cancellable_with_durable(script, config, IsolateCancel::new(), Some(session))
}

/// Evaluate an ESM durable task whose default export is
/// `async (ctx, input) => value`. `ctx` is the replay-safe durable API and
/// `input_json` is parsed once inside the isolate before invocation.
pub fn eval_durable_module(
    source: &str,
    input_json: &str,
    config: IsolateConfig,
    session: DurableSession,
) -> Result<Value, EngineError> {
    const MAX_INPUT_BYTES: usize = 1_048_576;
    if input_json.len() > MAX_INPUT_BYTES {
        return Err(EngineError::Isolate(format!(
            "durable task input exceeds {MAX_INPUT_BYTES} bytes"
        )));
    }
    serde_json::from_str::<serde_json::Value>(input_json)
        .map_err(|err| EngineError::Isolate(format!("invalid durable task input: {err}")))?;
    let source = source.to_owned();
    let input_json = input_json.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tysel-qjs".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_module_on_worker(&source, &input_json, config, session)
            }))
            .unwrap_or_else(|_| Err(EngineError::Isolate("quickjs worker panicked".into())));
            let _ = tx.send(result);
        })
        .map_err(|err| EngineError::Isolate(err.to_string()))?;
    rx.recv().map_err(|err| EngineError::Isolate(err.to_string()))?
}

fn eval_cancellable_with_durable(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    durable: Option<DurableSession>,
) -> Result<Value, EngineError> {
    let script = script.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tysel-qjs".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_on_worker(&script, config, cancel, durable)
            }))
            .unwrap_or_else(|_| Err(EngineError::Isolate("quickjs worker panicked".into())));
            let _ = tx.send(result);
        })
        .map_err(|err| EngineError::Isolate(err.to_string()))?;
    rx.recv().map_err(|err| EngineError::Isolate(err.to_string()))?
}

fn run_on_worker(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    durable: Option<DurableSession>,
) -> Result<Value, EngineError> {
    let request_deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
    let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
    let reactor = queue::spawn_reactor(cancel.flag(), request_deadline);
    run_with_reactor(
        Evaluation::Script(script),
        config,
        cancel,
        reactor,
        request_deadline,
        cpu,
        durable,
    )
}

fn run_module_on_worker(
    source: &str,
    input_json: &str,
    config: IsolateConfig,
    session: DurableSession,
) -> Result<Value, EngineError> {
    let input_json = session.record_input_json(input_json).map_err(EngineError::Isolate)?;
    let cancel = IsolateCancel::new();
    let request_deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
    let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
    let reactor = queue::spawn_reactor(cancel.flag(), request_deadline);
    run_with_reactor(
        Evaluation::DurableModule { source, input_json: &input_json },
        config,
        cancel,
        reactor,
        request_deadline,
        cpu,
        Some(session),
    )
}

/// Evaluate `script` using a caller-supplied I/O reactor (local or IPC proxy).
pub fn eval_with_reactor(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    reactor: queue::Reactor,
) -> Result<Value, EngineError> {
    let request_deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
    eval_with_reactor_deadline(script, config, cancel, reactor, request_deadline)
}

/// Evaluate `script` with a caller-supplied reactor and shared request deadline.
pub fn eval_with_reactor_deadline(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    reactor: queue::Reactor,
    request_deadline: Instant,
) -> Result<Value, EngineError> {
    let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
    run_with_reactor(
        Evaluation::Script(script),
        config,
        cancel,
        reactor,
        request_deadline,
        cpu,
        None,
    )
}

fn run_with_reactor(
    evaluation: Evaluation<'_>,
    config: IsolateConfig,
    cancel: IsolateCancel,
    reactor: queue::Reactor,
    request_deadline: Instant,
    cpu: Arc<CpuBudget>,
    durable: Option<DurableSession>,
) -> Result<Value, EngineError> {
    let cancel_flag = cancel.flag();

    let runtime = Runtime::new().map_err(js_err)?;
    runtime.set_memory_limit(config.memory_limit_bytes);
    {
        let cancel_flag = cancel_flag.clone();
        let cpu = cpu.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            cancel_flag.load(Ordering::SeqCst)
                || cpu.exhausted()
                || Instant::now() >= request_deadline
        })));
    }
    let context = Context::full(&runtime).map_err(js_err)?;
    let started_async = context.with(|ctx| {
        start_script(
            ctx,
            evaluation,
            reactor.io.clone(),
            &cancel,
            request_deadline,
            &cpu,
            durable.clone(),
        )
    })?;

    let result = match started_async {
        Some(value) => Ok(value),
        None => {
            wait_until_settled(
                &runtime,
                &context,
                &reactor,
                &cancel,
                request_deadline,
                &cpu,
                durable.as_ref(),
            )?;
            let settled = context
                .with(|ctx| take_settled(&ctx, &cancel, request_deadline, &cpu))?
                .ok_or_else(|| EngineError::Isolate("async script did not settle".into()))?;
            if evaluation.is_durable_module() {
                context.with(|ctx| {
                    let json = ctx
                        .globals()
                        .get::<_, String>("__tysel_task_value_json")
                        .map_err(js_err)?;
                    if json.len() > 1_048_576 {
                        return Err(EngineError::Isolate(
                            "durable task result exceeds 1048576 bytes".into(),
                        ));
                    }
                    let value: serde_json::Value = serde_json::from_str(&json)
                        .map_err(|err| EngineError::Isolate(err.to_string()))?;
                    Ok(from_json(value))
                })
            } else {
                Ok(settled)
            }
        }
    };

    if result.is_ok()
        && let Some(durable) = &durable
    {
        durable.ensure_consumed().map_err(EngineError::Isolate)?;
    }

    let _ = context.with(|ctx| {
        let _ = host::drop_host(&ctx);
        let _ = ctx.globals().remove("__tysel_result");
        let _ = ctx.globals().remove("__tysel_task_input_json");
        let _ = ctx.globals().remove("__tysel_durable_export");
        let _ = ctx.globals().remove("__tysel_task_value_json");
        Ok::<_, EngineError>(())
    });
    drop(context);
    runtime.set_interrupt_handler(None);
    runtime.run_gc();
    result
}

fn start_script(
    ctx: Ctx<'_>,
    evaluation: Evaluation<'_>,
    io: crate::queue::IoHandle,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu: &CpuBudget,
    durable: Option<DurableSession>,
) -> Result<Option<Value>, EngineError> {
    match durable {
        Some(durable) => host::install_durable(ctx.clone(), io, 0, durable).map_err(js_err)?,
        None => host::install(ctx.clone(), io, 0).map_err(js_err)?,
    }
    let Evaluation::Script(script) = evaluation else {
        let Evaluation::DurableModule { source, input_json } = evaluation else {
            unreachable!();
        };
        let (source, export_name) = split_durable_export(source);
        ctx.globals().set("__tysel_task_input_json", input_json).map_err(js_err)?;
        ctx.globals().set("__tysel_durable_export", export_name.unwrap_or("")).map_err(js_err)?;
        Module::declare(ctx.clone(), "tysel-task-input.js", DURABLE_INPUT_MODULE)
            .map_err(js_err)?;
        Module::declare(ctx.clone(), "app.js", source).map_err(js_err)?;
        let promise = Module::evaluate(ctx.clone(), "tysel-task-boot.js", BOOT_DURABLE_TASK)
            .map_err(js_err)?;
        ctx.globals().set("__tysel_result", promise).map_err(js_err)?;
        return Ok(None);
    };
    let evaluated = ctx
        .eval::<rquickjs::Value, _>(script)
        .map_err(|err| map_eval_error(err, cancel, request_deadline, cpu))?;
    if evaluated.is_promise() {
        ctx.globals().set("__tysel_result", evaluated).map_err(js_err)?;
        Ok(None)
    } else {
        from_js(&ctx, evaluated).map(Some)
    }
}

const DURABLE_INPUT_MODULE: &str = r#"
const input = JSON.parse(globalThis.__tysel_task_input_json);
delete globalThis.__tysel_task_input_json;
export default input;
"#;

pub(crate) const DURABLE_EXPORT_PREFIX: &str = "/*tysel-durable-export:";

pub(crate) fn split_durable_export(source: &str) -> (&str, Option<&str>) {
    let Some(rest) = source.strip_prefix(DURABLE_EXPORT_PREFIX) else {
        return (source, None);
    };
    let Some(end) = rest.find("*/") else {
        return (source, None);
    };
    let name = rest[..end].trim();
    let body = rest[end + 2..].trim_start_matches(['\r', '\n']);
    if name.is_empty() { (source, None) } else { (body, Some(name)) }
}

pub fn encode_durable_export(name: &str, source: &str) -> String {
    format!("{DURABLE_EXPORT_PREFIX}{name}*/\n{source}")
}

const BOOT_DURABLE_TASK: &str = r#"
import input from "tysel-task-input.js";
import task from "app.js";
const exportName = String(globalThis.__tysel_durable_export || "");
delete globalThis.__tysel_durable_export;
function resolve(exported) {
  if (typeof exported === "function") {
    if (exportName && exportName !== "default") {
      throw new TypeError("durable task module exports a default function, not " + exportName);
    }
    return exported;
  }
  const table = exported && exported.durable;
  if (!table || typeof table !== "object" || Array.isArray(table)) {
    throw new TypeError("durable task module must export a default function or durable map");
  }
  const name = exportName || Object.keys(table).sort()[0];
  const run = name ? table[name] : undefined;
  if (typeof run !== "function") {
    throw new TypeError("durable export is missing");
  }
  return run;
}
const value = await resolve(task)(globalThis.tysel.durable, input);
const encoded = JSON.stringify(value);
if (encoded === undefined) {
  throw new TypeError("durable task result must be JSON serializable");
}
globalThis.__tysel_task_value_json = encoded;
"#;

pub(crate) fn wait_until_settled(
    runtime: &Runtime,
    context: &Context,
    reactor: &queue::Reactor,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu: &CpuBudget,
    durable: Option<&DurableSession>,
) -> Result<(), EngineError> {
    loop {
        cpu.resume();
        drain_jobs(runtime)?;
        if context.with(|ctx| promise_is_settled(&ctx))? {
            return context.with(|ctx| match take_raw(&ctx) {
                Some(Err(err)) => Err(map_eval_error(err, cancel, request_deadline, cpu)),
                _ => Ok(()),
            });
        }
        if durable
            .map(DurableSession::is_suspended)
            .transpose()
            .map_err(EngineError::Isolate)?
            .unwrap_or(false)
        {
            return Err(EngineError::Suspended);
        }
        if let Some(reason) = wait_reason(cancel, request_deadline) {
            cancel.cancel();
            context.with(|ctx| host::reject_all(&ctx, reason).map_err(js_err))?;
            return Err(EngineError::Interrupted(reason));
        }
        cpu.pause();
        match reactor.completions.recv_timeout(Duration::from_millis(5)) {
            Ok(IoCompletion { id, result }) => {
                cpu.resume();
                let too_large = matches!(&result, Err(message) if message.contains("request body exceeds limit"));
                context.with(|ctx| host::settle(&ctx, id, result).map_err(js_err))?;
                if too_large {
                    return Err(EngineError::BodyTooLarge);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                cpu.resume();
                drain_jobs(runtime)?;
                if context.with(|ctx| promise_is_settled(&ctx))? {
                    return Ok(());
                }
                return Err(EngineError::Isolate("io reactor stopped".into()));
            }
        }
    }
}

fn promise_is_settled(ctx: &Ctx<'_>) -> Result<bool, EngineError> {
    let promise: Promise = ctx.globals().get("__tysel_result").map_err(js_err)?;
    Ok(promise.result::<rquickjs::Value>().is_some())
}

fn take_raw<'js>(ctx: &Ctx<'js>) -> Option<rquickjs::Result<rquickjs::Value<'js>>> {
    let Ok(promise) = ctx.globals().get::<_, Promise>("__tysel_result") else {
        return None;
    };
    promise.result()
}

fn take_settled(
    ctx: &Ctx<'_>,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu: &CpuBudget,
) -> Result<Option<Value>, EngineError> {
    let promise: Promise = ctx.globals().get("__tysel_result").map_err(js_err)?;
    match promise.result::<rquickjs::Value>() {
        Some(Ok(value)) => from_js(ctx, value).map(Some),
        Some(Err(err)) => Err(map_eval_error(err, cancel, request_deadline, cpu)),
        None => Ok(None),
    }
}

pub(crate) fn drain_jobs(runtime: &Runtime) -> Result<(), EngineError> {
    loop {
        match runtime.execute_pending_job() {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(err) => return Err(EngineError::Isolate(err.to_string())),
        }
    }
}

pub(crate) fn wait_reason(
    cancel: &IsolateCancel,
    request_deadline: Instant,
) -> Option<InterruptReason> {
    if cancel.0.load(Ordering::SeqCst) {
        return Some(InterruptReason::Cancelled);
    }
    if Instant::now() >= request_deadline {
        return Some(InterruptReason::Timeout);
    }
    None
}

fn from_js(ctx: &Ctx<'_>, value: rquickjs::Value<'_>) -> Result<Value, EngineError> {
    if value.is_null() || value.is_undefined() {
        return Ok(Value::Null);
    }
    if let Some(v) = value.as_bool() {
        return Ok(Value::Bool(v));
    }
    if let Some(v) = value.as_number() {
        return Ok(Value::Number(v));
    }
    if let Some(v) = value.as_string() {
        return Ok(Value::String(v.to_string().map_err(js_err)?));
    }
    let _ = ctx;
    Err(EngineError::Isolate(format!("unsupported js type: {}", value.type_name())))
}

pub(crate) fn from_json(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(value),
        serde_json::Value::Number(value) => Value::Number(value.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(value) => Value::String(value),
        serde_json::Value::Array(values) => {
            Value::Array(values.into_iter().map(from_json).collect())
        }
        serde_json::Value::Object(fields) => {
            Value::Record(fields.into_iter().map(|(key, value)| (key, from_json(value))).collect())
        }
    }
}

pub(crate) fn map_eval_error(
    err: rquickjs::Error,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu: &CpuBudget,
) -> EngineError {
    if cancel.0.load(Ordering::SeqCst) {
        return EngineError::Interrupted(InterruptReason::Cancelled);
    }
    if Instant::now() >= request_deadline || cpu.exhausted() {
        return EngineError::Interrupted(InterruptReason::Timeout);
    }
    let message = err.to_string();
    if message.contains("request body exceeds limit") {
        return EngineError::BodyTooLarge;
    }
    if message.to_ascii_lowercase().contains("out of memory")
        || message.to_ascii_lowercase().contains("memory")
    {
        return EngineError::Interrupted(InterruptReason::MemoryLimit);
    }
    EngineError::Isolate(message)
}

pub(crate) fn js_err(err: impl std::fmt::Display) -> EngineError {
    EngineError::Isolate(err.to_string())
}
