use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rquickjs::{Context, Function, Runtime};
use tokio::sync::{mpsc, oneshot};
use tysel_engine::{EngineError, HttpHead, HttpRequest, IsolateConfig};

use crate::cpu::CpuBudget;
use crate::fetch;
use crate::host;
use crate::isolate::{self, IsolateCancel};
use crate::queue;

struct Job {
    request: HttpRequest,
    head_tx: oneshot::Sender<Result<HttpHead, EngineError>>,
    body_tx: mpsc::Sender<Vec<u8>>,
}

struct Budgets {
    cpu: Arc<CpuBudget>,
    request: Instant,
}

pub struct IsolatePool {
    workers: Vec<mpsc::Sender<Job>>,
    next: Arc<AtomicUsize>,
    threads: Vec<JoinHandle<()>>,
}

impl IsolatePool {
    pub fn spawn(workers: usize, source: &str, config: IsolateConfig) -> Result<Self, EngineError> {
        let workers = workers.max(1);
        let mut senders = Vec::with_capacity(workers);
        let mut threads = Vec::with_capacity(workers);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        for id in 0..workers {
            let (tx, rx) = mpsc::channel(32);
            let source = source.to_owned();
            let ready_tx = ready_tx.clone();
            let thread = std::thread::Builder::new()
                .name(format!("tysel-qjs-{id}"))
                .spawn(move || worker_loop(id as u32, source, config, rx, ready_tx))
                .map_err(|err| EngineError::Isolate(err.to_string()))?;
            senders.push(tx);
            threads.push(thread);
        }
        drop(ready_tx);
        for _ in 0..workers {
            ready_rx
                .recv()
                .map_err(|_| EngineError::Isolate("isolate worker failed to start".into()))??;
        }
        Ok(Self { workers: senders, next: Arc::new(AtomicUsize::new(0)), threads })
    }

    pub async fn dispatch(
        &self,
        request: HttpRequest,
    ) -> Result<(HttpHead, mpsc::Receiver<Vec<u8>>), EngineError> {
        let (head_tx, head_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::channel(16);
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        self.workers[index]
            .send(Job { request, head_tx, body_tx })
            .await
            .map_err(|_| EngineError::Isolate("isolate worker stopped".into()))?;
        match head_rx.await {
            Ok(Ok(head)) => Ok((head, body_rx)),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(EngineError::Isolate("isolate dropped the response head".into())),
        }
    }
}

impl Drop for IsolatePool {
    fn drop(&mut self) {
        self.workers.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn worker_loop(
    id: u32,
    source: String,
    config: IsolateConfig,
    mut jobs: mpsc::Receiver<Job>,
    ready: std::sync::mpsc::Sender<Result<(), EngineError>>,
) {
    let cancel = IsolateCancel::new();
    if let Err(err) = run_worker(id, &source, config, cancel, &mut jobs, &ready) {
        let _ = ready.send(Err(EngineError::Isolate(err.to_string())));
        while let Ok(job) = jobs.try_recv() {
            let _ = job.head_tx.send(Err(EngineError::Isolate(err.to_string())));
        }
    }
}

fn run_worker(
    id: u32,
    source: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    jobs: &mut mpsc::Receiver<Job>,
    ready: &std::sync::mpsc::Sender<Result<(), EngineError>>,
) -> Result<(), EngineError> {
    let budgets = Arc::new(Mutex::new(Budgets {
        cpu: CpuBudget::new(Duration::from_secs(60)),
        request: Instant::now() + Duration::from_secs(60),
    }));
    let runtime = Runtime::new().map_err(isolate::js_err)?;
    runtime.set_memory_limit(config.memory_limit_bytes);
    {
        let budgets = budgets.clone();
        let cancel_flag = cancel.flag();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                return true;
            }
            let budgets = budgets.lock().expect("budgets");
            budgets.cpu.exhausted() || Instant::now() >= budgets.request
        })));
    }

    let reactor = queue::spawn_reactor_until_cancel(cancel.flag());
    let context = Context::full(&runtime).map_err(isolate::js_err)?;
    let load_cpu = CpuBudget::new(Duration::from_secs(5));
    context.with(|ctx| {
        fetch::install_web_api(ctx.clone())?;
        host::install(ctx.clone(), reactor.io.clone(), id).map_err(isolate::js_err)?;
        fetch::load_fetch_handler(ctx, source)
    })?;
    isolate::wait_until_settled(
        &runtime,
        &context,
        &reactor,
        &cancel,
        Instant::now() + Duration::from_secs(5),
        &load_cpu,
    )?;
    context.with(|ctx| {
        let _: Function = ctx.globals().get("__tysel_fetch").map_err(isolate::js_err)?;
        Ok::<_, EngineError>(())
    })?;
    let _ = ready.send(Ok(()));

    while let Some(job) = jobs.blocking_recv() {
        let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
        let request_deadline =
            Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
        *budgets.lock().expect("budgets") = Budgets { cpu: cpu.clone(), request: request_deadline };
        if let Err(err) =
            handle_job(&runtime, &context, &reactor, &cancel, job, request_deadline, &cpu)
        {
            let _ = err;
        }
        let _ = context.with(|ctx| ctx.globals().remove("__tysel_response"));
    }

    let _ = context.with(|ctx| {
        let _ = host::drop_host(&ctx);
        let _ = ctx.globals().remove("__tysel_fetch");
        let _ = ctx.globals().remove("__tysel_result");
        Ok::<_, EngineError>(())
    });
    drop(context);
    runtime.set_interrupt_handler(None);
    runtime.run_gc();
    Ok(())
}

fn handle_job(
    runtime: &Runtime,
    context: &Context,
    reactor: &queue::Reactor,
    cancel: &IsolateCancel,
    job: Job,
    request_deadline: Instant,
    cpu: &CpuBudget,
) -> Result<(), EngineError> {
    let Job { request, head_tx, body_tx } = job;
    let pending = match context.with(|ctx| fetch::begin_fetch(ctx, &request)) {
        Ok(pending) => pending,
        Err(err) => {
            let _ = head_tx.send(Err(EngineError::Isolate(err.to_string())));
            return Err(err);
        }
    };
    if pending {
        if let Err(err) =
            isolate::wait_until_settled(runtime, context, reactor, cancel, request_deadline, cpu)
        {
            let _ = head_tx.send(Err(EngineError::Isolate(err.to_string())));
            return Err(err);
        }
        if let Err(err) = context.with(fetch::take_response_into_globals) {
            let _ = head_tx.send(Err(EngineError::Isolate(err.to_string())));
            return Err(err);
        }
    }
    context.with(|ctx| fetch::emit_response(ctx, head_tx, body_tx))
}
