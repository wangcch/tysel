use std::collections::HashSet;
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
use crate::task_module::{ModuleMetadata, read_module_metadata};

pub struct IncomingHttp {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: mpsc::Receiver<Result<Vec<u8>, String>>,
    pub ws_in: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
    pub ws_out: Option<mpsc::Sender<Vec<u8>>>,
    pub request_id: u64,
}

impl From<HttpRequest> for IncomingHttp {
    fn from(request: HttpRequest) -> Self {
        Self {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: sealed_body(request.body),
            ws_in: None,
            ws_out: None,
            request_id: request.request_id,
        }
    }
}

struct Job {
    request: IncomingHttp,
    response_tx: ResponseSender,
    _pending: PendingJob,
}

struct ActiveJob {
    request: IncomingHttp,
    response_tx: ResponseSender,
}

struct PendingJob {
    counts: Arc<[AtomicUsize]>,
    worker: usize,
}

impl Drop for PendingJob {
    fn drop(&mut self) {
        self.counts[self.worker].fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) struct PreparedHttpResponse {
    pub head: HttpHead,
    pub body: OutgoingHttpBody,
}

pub(crate) type ResponseSender = oneshot::Sender<Result<PreparedHttpResponse, EngineError>>;

pub enum OutgoingHttpBody {
    Buffered(Vec<u8>),
    Stream(mpsc::Receiver<Vec<u8>>),
}

struct Budgets {
    cpu: Arc<CpuBudget>,
    request: Instant,
}

pub struct IsolatePool {
    workers: Vec<WorkerSlot>,
    pending: Arc<[AtomicUsize]>,
    next: Arc<AtomicUsize>,
    threads: Vec<JoinHandle<()>>,
}

struct WorkerSlot {
    jobs: mpsc::Sender<Job>,
}

impl IsolatePool {
    pub fn spawn(workers: usize, source: &str, config: IsolateConfig) -> Result<Self, EngineError> {
        Self::spawn_with_metadata(workers, source, config).map(|(pool, _)| pool)
    }

    pub fn spawn_with_metadata(
        workers: usize,
        source: &str,
        config: IsolateConfig,
    ) -> Result<(Self, ModuleMetadata), EngineError> {
        let workers = workers.max(1);
        let mut senders = Vec::with_capacity(workers);
        let mut threads = Vec::with_capacity(workers);
        let pending: Arc<[AtomicUsize]> =
            (0..workers).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>().into();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        for id in 0..workers {
            let (tx, rx) = mpsc::channel(32);
            let source = source.to_owned();
            let ready_tx = ready_tx.clone();
            let thread = std::thread::Builder::new()
                .name(format!("tysel-qjs-{id}"))
                .spawn(move || worker_loop(id as u32, source, config, rx, ready_tx))
                .map_err(|err| EngineError::Isolate(err.to_string()))?;
            senders.push(WorkerSlot { jobs: tx });
            threads.push(thread);
        }
        drop(ready_tx);
        let mut metadata = None;
        for _ in 0..workers {
            let worker_metadata = ready_rx
                .recv()
                .map_err(|_| EngineError::Isolate("isolate worker failed to start".into()))??;
            if metadata.is_none() {
                metadata = Some(worker_metadata);
            }
        }
        let pool = Self { workers: senders, pending, next: Arc::new(AtomicUsize::new(0)), threads };
        Ok((pool, metadata.expect("at least one isolate worker")))
    }

    pub async fn dispatch(
        &self,
        request: HttpRequest,
    ) -> Result<(HttpHead, mpsc::Receiver<Vec<u8>>), EngineError> {
        self.dispatch_incoming(IncomingHttp::from(request)).await
    }

    pub async fn dispatch_incoming(
        &self,
        request: IncomingHttp,
    ) -> Result<(HttpHead, mpsc::Receiver<Vec<u8>>), EngineError> {
        let (head, body) = self.dispatch_response(request).await?;
        let body = match body {
            OutgoingHttpBody::Buffered(bytes) => sealed_response_body(bytes),
            OutgoingHttpBody::Stream(body) => body,
        };
        Ok((head, body))
    }

    pub async fn dispatch_response(
        &self,
        request: IncomingHttp,
    ) -> Result<(HttpHead, OutgoingHttpBody), EngineError> {
        let (response_tx, response_rx) = oneshot::channel();
        let mut request = Some(request);
        let mut response_tx = Some(response_tx);
        loop {
            let (index, pending) = self
                .reserve_worker()
                .ok_or_else(|| EngineError::Isolate("all isolate workers stopped".into()))?;
            let job = Job {
                request: request.take().expect("request is retained until dispatch"),
                response_tx: response_tx
                    .take()
                    .expect("response sender is retained until dispatch"),
                _pending: pending,
            };
            match self.workers[index].jobs.send(job).await {
                Ok(()) => break,
                Err(error) => {
                    let Job {
                        request: returned_request,
                        response_tx: returned_response_tx,
                        _pending: _,
                    } = error.0;
                    request = Some(returned_request);
                    response_tx = Some(returned_response_tx);
                }
            }
        }
        match response_rx.await {
            Ok(Ok(PreparedHttpResponse { head, body })) => Ok((head, body)),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(EngineError::Isolate("isolate dropped the response head".into())),
        }
    }

    fn reserve_worker(&self) -> Option<(usize, PendingJob)> {
        let start = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        let index = (0..self.workers.len())
            .map(|offset| (start + offset) % self.workers.len())
            .filter(|&index| !self.workers[index].jobs.is_closed())
            .min_by_key(|&index| self.pending[index].load(Ordering::Relaxed))?;
        self.pending[index].fetch_add(1, Ordering::Relaxed);
        Some((index, PendingJob { counts: self.pending.clone(), worker: index }))
    }

    #[cfg(test)]
    pub(crate) fn pending_jobs(&self) -> Vec<usize> {
        self.pending.iter().map(|count| count.load(Ordering::Relaxed)).collect()
    }

    /// Run one request to completion on the process I/O runtime. Isolated
    /// workers use this from a blocking IPC thread.
    pub fn dispatch_sync(&self, request: HttpRequest) -> Result<(HttpHead, Vec<u8>), EngineError> {
        crate::queue::io_handle().block_on(async {
            let (head, mut chunks) = self.dispatch(request).await?;
            let mut body = Vec::new();
            while let Some(chunk) = chunks.recv().await {
                body.extend(chunk);
            }
            Ok((head, body))
        })
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
    ready: std::sync::mpsc::Sender<Result<ModuleMetadata, EngineError>>,
) {
    let cancel = IsolateCancel::new();
    if let Err(err) = run_worker(id, &source, config, cancel, &mut jobs, &ready) {
        let _ = ready.send(Err(EngineError::Isolate(err.to_string())));
        while let Ok(job) = jobs.try_recv() {
            let _ = job.response_tx.send(Err(EngineError::Isolate(err.to_string())));
        }
    }
}

fn run_worker(
    id: u32,
    source: &str,
    config: IsolateConfig,
    cancel: IsolateCancel,
    jobs: &mut mpsc::Receiver<Job>,
    ready: &std::sync::mpsc::Sender<Result<ModuleMetadata, EngineError>>,
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
        None,
    )?;
    let metadata = context.with(|ctx| {
        let _: Function = ctx.globals().get("__tysel_fetch").map_err(isolate::js_err)?;
        let metadata = read_module_metadata(ctx.clone())?;
        let _ = ctx.globals().remove("__tysel_task_manifest_json");
        let _ = ctx.globals().remove("__tysel_durable_exports_json");
        Ok::<_, EngineError>(metadata)
    })?;
    teardown_scope(&context, &reactor, 0, Duration::from_secs(5))?;
    let _ = ready.send(Ok(metadata));

    while let Some(job) = jobs.blocking_recv() {
        let Job { request, response_tx, _pending: pending } = job;
        let request_id = request.request_id;
        let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
        let request_deadline =
            Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
        *budgets.lock().expect("budgets") = Budgets { cpu: cpu.clone(), request: request_deadline };
        let job_result = handle_job(
            &runtime,
            &context,
            &reactor,
            &cancel,
            ActiveJob { request, response_tx },
            request_deadline,
            &cpu,
        );
        teardown_scope(
            &context,
            &reactor,
            request_id,
            Duration::from_millis(config.request_timeout_ms.max(100)),
        )?;
        let _ = job_result;
        drop(pending);
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

fn teardown_scope(
    context: &Context,
    reactor: &queue::Reactor,
    request_id: u64,
    quiesce_timeout: Duration,
) -> Result<(), EngineError> {
    reactor.io.bind_request(0);
    let abandoned = reactor.io.cancel_request(request_id);
    let _ = context.with(|ctx| host::discard_operations(&ctx, &abandoned));
    reactor.io.inbound.clear();
    reactor.io.outbound.clear_all();
    reactor.io.ws_in.clear();
    reactor.io.ws_out.clear();
    reactor.io.client_ws.clear();
    let _ = context.with(|ctx| {
        let _ = host::reset_timers(&ctx);
        let _ = ctx.globals().remove("__tysel_response");
        let _ = ctx.globals().remove("__tysel_result");
        let _ = ctx.globals().remove("__tysel_ws_done");
        let _ = ctx.globals().set("__tysel_ws_accepted", false);
        let generation = ctx
            .globals()
            .get::<_, u64>("__tysel_request_generation")
            .unwrap_or(0)
            .saturating_add(1);
        let _ = ctx.globals().set("__tysel_request_generation", generation);
        Ok::<_, EngineError>(())
    });
    let mut remaining = abandoned.into_iter().collect::<HashSet<_>>();
    let deadline = Instant::now() + quiesce_timeout;
    while !remaining.is_empty() {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(EngineError::Isolate(format!(
                "{} native operations did not quiesce after request {request_id}",
                remaining.len()
            )));
        }
        match reactor.completions.recv_timeout(timeout.min(Duration::from_millis(10))) {
            Ok(queue::IoCompletion { id, .. }) => {
                reactor.io.finish(id);
                remaining.remove(&id);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(EngineError::Isolate(
                    "io reactor stopped while quiescing request operations".into(),
                ));
            }
        }
    }
    Ok(())
}

fn sealed_body(bytes: Vec<u8>) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (tx, rx) = mpsc::channel(1);
    if !bytes.is_empty() {
        let _ = tx.try_send(Ok(bytes));
    }
    rx
}

fn sealed_response_body(bytes: Vec<u8>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel(1);
    if !bytes.is_empty() {
        let _ = tx.try_send(bytes);
    }
    rx
}

fn handle_job(
    runtime: &Runtime,
    context: &Context,
    reactor: &queue::Reactor,
    cancel: &IsolateCancel,
    job: ActiveJob,
    request_deadline: Instant,
    cpu: &CpuBudget,
) -> Result<(), EngineError> {
    let ActiveJob { request, response_tx } = job;
    reactor.io.bind_request(request.request_id);
    reactor.io.inbound.install(request.body);
    if let Some(ws_in) = request.ws_in {
        reactor.io.ws_in.install(ws_in);
    }
    if let Some(ws_out) = request.ws_out {
        reactor.io.ws_out.install(ws_out);
    }
    let _ = context.with(|ctx| ctx.globals().set("__tysel_ws_accepted", false));
    let pending = match context.with(|ctx| {
        fetch::begin_fetch(
            ctx,
            &HttpRequest {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: Vec::new(),
                request_id: request.request_id,
            },
        )
    }) {
        Ok(pending) => pending,
        Err(err) => {
            let _ = response_tx.send(Err(err.clone()));
            return Err(err);
        }
    };
    if pending {
        if let Err(err) = isolate::wait_until_settled(
            runtime,
            context,
            reactor,
            cancel,
            request_deadline,
            cpu,
            None,
        ) {
            let _ = response_tx.send(Err(err.clone()));
            return Err(err);
        }
        if let Err(err) = context.with(fetch::take_response_into_globals) {
            let _ = response_tx.send(Err(err.clone()));
            return Err(err);
        }
    }
    context.with(|ctx| fetch::emit_response(ctx, response_tx))?;
    if context.with(fetch::arm_websocket)? {
        isolate::wait_until_settled(
            runtime,
            context,
            reactor,
            cancel,
            request_deadline,
            cpu,
            None,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pool(senders: Vec<mpsc::Sender<Job>>) -> IsolatePool {
        let pending = (0..senders.len()).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>().into();
        IsolatePool {
            workers: senders.into_iter().map(|jobs| WorkerSlot { jobs }).collect(),
            pending,
            next: Arc::new(AtomicUsize::new(0)),
            threads: Vec::new(),
        }
    }

    #[test]
    fn scheduler_skips_stopped_workers() {
        let (stopped_tx, stopped_rx) = mpsc::channel(1);
        drop(stopped_rx);
        let (healthy_tx, _healthy_rx) = mpsc::channel(1);
        let pool = test_pool(vec![stopped_tx, healthy_tx]);

        let (worker, pending) = pool.reserve_worker().expect("healthy worker remains");
        assert_eq!(worker, 1);
        assert_eq!(pool.pending_jobs(), vec![0, 1]);
        drop(pending);
        assert_eq!(pool.pending_jobs(), vec![0, 0]);
    }

    #[test]
    fn scheduler_reports_when_all_workers_stopped() {
        let (first_tx, first_rx) = mpsc::channel(1);
        let (second_tx, second_rx) = mpsc::channel(1);
        drop(first_rx);
        drop(second_rx);
        let pool = test_pool(vec![first_tx, second_tx]);

        assert!(pool.reserve_worker().is_none());
    }

    #[test]
    fn startup_returns_metadata_from_the_http_module_evaluation() {
        let source = r#"
export default {
  tasks: {
    events: { kind: "queue", name: "events", async handler() {} },
  },
  durable: {
    workflow() {},
  },
  fetch() { return new Response("ok"); },
};
"#;
        let (_pool, metadata) =
            IsolatePool::spawn_with_metadata(1, source, IsolateConfig::default()).unwrap();
        assert_eq!(metadata.task_definitions.len(), 1);
        assert_eq!(metadata.task_definitions[0].name, "events");
        assert_eq!(metadata.durable_exports, ["workflow"]);
    }

    #[test]
    fn startup_preserves_default_function_durable_metadata() {
        let source = r#"
async function workflow() {}
workflow.fetch = function() { return new Response("ok"); };
export default workflow;
"#;
        let (_pool, metadata) =
            IsolatePool::spawn_with_metadata(1, source, IsolateConfig::default()).unwrap();
        assert!(metadata.task_definitions.is_empty());
        assert_eq!(metadata.durable_exports, ["default"]);
    }
}
