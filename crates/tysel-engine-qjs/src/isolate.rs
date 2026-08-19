use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Promise, Runtime};
use tysel_engine::{EngineError, InterruptReason, IsolateConfig, Value};

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

    fn flag(&self) -> Arc<AtomicBool> {
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
    let script = script.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tysel-qjs".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_on_worker(&script, config, cancel)
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
) -> Result<Value, EngineError> {
    let started = Instant::now();
    let cpu_deadline = started + Duration::from_millis(config.cpu_ms_per_turn.max(1));
    let request_deadline = started + Duration::from_millis(config.request_timeout_ms.max(1));
    let cancel_flag = cancel.flag();

    let runtime = Runtime::new().map_err(js_err)?;
    runtime.set_memory_limit(config.memory_limit_bytes);
    {
        let cancel_flag = cancel_flag.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            cancel_flag.load(Ordering::SeqCst)
                || Instant::now() >= cpu_deadline
                || Instant::now() >= request_deadline
        })));
    }

    let reactor = queue::spawn_reactor(cancel.flag(), request_deadline);
    let context = Context::full(&runtime).map_err(js_err)?;
    let started_async = context.with(|ctx| {
        start_script(ctx, script, reactor.io.clone(), &cancel, request_deadline, cpu_deadline)
    })?;

    let result = match started_async {
        Some(value) => Ok(value),
        None => {
            wait_for_result(&runtime, &context, &reactor, &cancel, request_deadline, cpu_deadline)
        }
    };

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
    cpu_deadline: Instant,
) -> Result<Option<Value>, EngineError> {
    host::install(ctx.clone(), io).map_err(js_err)?;
    let evaluated = ctx
        .eval::<rquickjs::Value, _>(script)
        .map_err(|err| map_eval_error(err, cancel, request_deadline, cpu_deadline))?;
    if evaluated.is_promise() {
        ctx.globals().set("__tysel_result", evaluated).map_err(js_err)?;
        Ok(None)
    } else {
        from_js(&ctx, evaluated).map(Some)
    }
}

fn wait_for_result(
    runtime: &Runtime,
    context: &Context,
    reactor: &queue::Reactor,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu_deadline: Instant,
) -> Result<Value, EngineError> {
    loop {
        drain_jobs(runtime)?;
        if let Some(value) =
            context.with(|ctx| take_settled(&ctx, cancel, request_deadline, cpu_deadline))?
        {
            return Ok(value);
        }
        if let Some(reason) = wait_reason(cancel, request_deadline) {
            context.with(|ctx| host::reject_all(&ctx, reason).map_err(js_err))?;
            return Err(EngineError::Interrupted(reason));
        }
        match reactor.completions.recv_timeout(Duration::from_millis(5)) {
            Ok(IoCompletion { id, result }) => {
                context.with(|ctx| host::settle(&ctx, id, result).map_err(js_err))?;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                drain_jobs(runtime)?;
                if let Some(value) = context
                    .with(|ctx| take_settled(&ctx, cancel, request_deadline, cpu_deadline))?
                {
                    return Ok(value);
                }
                return Err(EngineError::Isolate("io reactor stopped".into()));
            }
        }
    }
}

fn take_settled(
    ctx: &Ctx<'_>,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu_deadline: Instant,
) -> Result<Option<Value>, EngineError> {
    let promise: Promise = ctx.globals().get("__tysel_result").map_err(js_err)?;
    match promise.result::<rquickjs::Value>() {
        Some(Ok(value)) => from_js(ctx, value).map(Some),
        Some(Err(err)) => Err(map_eval_error(err, cancel, request_deadline, cpu_deadline)),
        None => Ok(None),
    }
}

fn drain_jobs(runtime: &Runtime) -> Result<(), EngineError> {
    loop {
        match runtime.execute_pending_job() {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(err) => return Err(EngineError::Isolate(err.to_string())),
        }
    }
}

fn wait_reason(cancel: &IsolateCancel, request_deadline: Instant) -> Option<InterruptReason> {
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

fn map_eval_error(
    err: rquickjs::Error,
    cancel: &IsolateCancel,
    request_deadline: Instant,
    cpu_deadline: Instant,
) -> EngineError {
    if cancel.0.load(Ordering::SeqCst) {
        return EngineError::Interrupted(InterruptReason::Cancelled);
    }
    if Instant::now() >= request_deadline || Instant::now() >= cpu_deadline {
        return EngineError::Interrupted(InterruptReason::Timeout);
    }
    let message = err.to_string();
    if message.to_ascii_lowercase().contains("out of memory")
        || message.to_ascii_lowercase().contains("memory")
    {
        return EngineError::Interrupted(InterruptReason::MemoryLimit);
    }
    EngineError::Isolate(message)
}

fn js_err(err: impl std::fmt::Display) -> EngineError {
    EngineError::Isolate(err.to_string())
}
