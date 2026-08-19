use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tysel_engine::{InterruptReason, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OpId(pub u64);

#[derive(Debug)]
pub enum IoRequest {
    Sleep { id: OpId, millis: u64 },
    Echo { id: OpId, value: String },
}

#[derive(Debug)]
pub struct IoCompletion {
    pub id: OpId,
    pub result: Result<Value, InterruptReason>,
}

#[derive(Clone)]
pub struct IoHandle {
    tx: UnboundedSender<IoRequest>,
    next_id: Arc<AtomicU64>,
}

impl IoHandle {
    pub fn submit(&self, request: impl FnOnce(OpId) -> IoRequest) -> OpId {
        let id = OpId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let _ = self.tx.send(request(id));
        id
    }
}

pub struct Reactor {
    pub io: IoHandle,
    pub completions: std::sync::mpsc::Receiver<IoCompletion>,
}

pub fn spawn_reactor(cancel: Arc<AtomicBool>, deadline: Instant) -> Reactor {
    let (req_tx, req_rx) = unbounded_channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tysel-io".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_time()
                .thread_name("tysel-io-worker")
                .build()
                .expect("tokio runtime");
            runtime.block_on(run_reactor(req_rx, done_tx, cancel, deadline));
        })
        .expect("spawn io reactor");

    Reactor {
        io: IoHandle { tx: req_tx, next_id: Arc::new(AtomicU64::new(1)) },
        completions: done_rx,
    }
}

pub fn spawn_reactor_until_cancel(cancel: Arc<AtomicBool>) -> Reactor {
    spawn_reactor(cancel, Instant::now() + Duration::from_secs(60 * 60 * 24 * 365))
}

async fn run_reactor(
    mut requests: UnboundedReceiver<IoRequest>,
    completions: std::sync::mpsc::Sender<IoCompletion>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
) {
    while let Some(request) = requests.recv().await {
        let completions = completions.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let completion = execute(request, cancel, deadline).await;
            let _ = completions.send(completion);
        });
    }
}

async fn execute(request: IoRequest, cancel: Arc<AtomicBool>, deadline: Instant) -> IoCompletion {
    match request {
        IoRequest::Sleep { id, millis } => IoCompletion {
            id,
            result: wait(Duration::from_millis(millis), &cancel, deadline).await,
        },
        IoRequest::Echo { id, value } => {
            let wait_result = wait(Duration::from_millis(1), &cancel, deadline).await;
            IoCompletion { id, result: wait_result.map(|_| Value::String(value)) }
        }
    }
}

async fn wait(
    duration: Duration,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Value, InterruptReason> {
    let sleep_until = Instant::now() + duration;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(InterruptReason::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(InterruptReason::Timeout);
        }
        let now = Instant::now();
        if now >= sleep_until {
            return Ok(Value::Null);
        }
        let slice = (sleep_until - now)
            .min(deadline.saturating_duration_since(now))
            .min(Duration::from_millis(5));
        tokio::time::sleep(slice).await;
    }
}
