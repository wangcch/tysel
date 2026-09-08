//! One sleeping watchdog per worker, independent of the blocking IPC reader.
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tysel_engine::InterruptReason;

type Shared = Arc<(Mutex<State>, Condvar)>;
#[derive(Default)]
struct State {
    stop: bool,
    deadline: Option<Instant>,
    cancelled: Option<Arc<AtomicBool>>,
    reason: Option<InterruptReason>,
}

pub(crate) struct Watchdog {
    shared: Shared,
    thread: Option<JoinHandle<()>>,
    child: Arc<Mutex<Child>>,
}

pub(crate) struct Operation {
    shared: Shared,
    child: Arc<Mutex<Child>>,
}

fn interrupt(state: &mut State, child: &Mutex<Child>) {
    let reason = if state.cancelled.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
        Some(InterruptReason::Cancelled)
    } else if state.deadline.is_some_and(|d| Instant::now() >= d) {
        Some(InterruptReason::Timeout)
    } else {
        None
    };
    if let Some(reason) = reason {
        // Serialize disarming with killing: a late watchdog must never kill the
        // next operation. The handle also prevents accidental PID reuse.
        let _ = child.lock().unwrap().kill();
        state.reason = Some(reason);
        state.deadline = None;
        state.cancelled = None;
    }
}

impl Watchdog {
    pub(crate) fn new(child: Arc<Mutex<Child>>) -> std::io::Result<Self> {
        let shared: Shared = Arc::default();
        let worker_state = shared.clone();
        let worker_child = child.clone();
        let thread =
            std::thread::Builder::new().name("tysel-ipc-watchdog".into()).spawn(move || {
                let (lock, wake) = &*worker_state;
                let mut state = lock.lock().unwrap();
                while !state.stop {
                    interrupt(&mut state, &worker_child);
                    state = match state.deadline {
                        Some(deadline) => {
                            let mut wait = deadline.saturating_duration_since(Instant::now());
                            if state.cancelled.is_some() {
                                wait = wait.min(Duration::from_millis(10));
                            }
                            wake.wait_timeout(state, wait).unwrap().0
                        }
                        None => wake.wait(state).unwrap(),
                    };
                }
            })?;
        Ok(Self { shared, thread: Some(thread), child })
    }

    pub(crate) fn arm(&self, deadline: Instant, cancelled: Option<Arc<AtomicBool>>) -> Operation {
        let mut state = self.shared.0.lock().unwrap();
        state.deadline = Some(deadline);
        state.cancelled = cancelled;
        state.reason = None;
        interrupt(&mut state, &self.child);
        self.shared.1.notify_one();
        Operation { shared: self.shared.clone(), child: self.child.clone() }
    }
}

impl Operation {
    pub(crate) fn finish(self) -> Option<InterruptReason> {
        let mut state = self.shared.0.lock().unwrap();
        interrupt(&mut state, &self.child);
        state.deadline = None;
        state.cancelled = None;
        state.reason
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        let mut state = self.shared.0.lock().unwrap();
        interrupt(&mut state, &self.child);
        state.deadline = None;
        state.cancelled = None;
        self.shared.1.notify_one();
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        {
            let mut state = self.shared.0.lock().unwrap();
            state.stop = true;
            self.shared.1.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
