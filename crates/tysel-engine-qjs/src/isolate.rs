use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Promise, Runtime};
use tysel_engine::{EngineError, InterruptReason, IsolateConfig, Value};

use crate::cpu::CpuBudget;
use crate::durable::DurableSession;
use crate::host;
use crate::queue::{self, IoCompletion};

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
    run_with_reactor(script, config, cancel, reactor, request_deadline, cpu, durable)
}

/// Evaluate `script` using a caller-supplied I/O reactor (local or IPC proxy).
pub fn eval_with_reactor(
    script: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    reactor: queue::Reactor,
) -> Result<Value, EngineError> {
    let request_deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
    let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
    run_with_reactor(script, config, cancel, reactor, request_deadline, cpu, None)
}

fn run_with_reactor(
    script: &str,
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
            script,
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
            wait_until_settled(&runtime, &context, &reactor, &cancel, request_deadline, &cpu)?;
            context
                .with(|ctx| take_settled(&ctx, &cancel, request_deadline, &cpu))?
                .ok_or_else(|| EngineError::Isolate("async script did not settle".into()))
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
        Ok::<_, EngineError>(())
    });
    drop(context);
    runtime.set_interrupt_handler(None);
    runtime.run_gc();
    result
}

fn start_script(
    ctx: Ctx<'_>,
    script: &str,
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

pub(crate) fn wait_until_settled(
    runtime: &Runtime,
    context: &Context,
    reactor: &queue::Reactor,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu: &CpuBudget,
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
