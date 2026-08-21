use std::sync::{Arc, OnceLock, RwLock};

type StartFn = Box<dyn Fn(&str, &str) -> Result<String, String> + Send + Sync>;
type SignalFn = Box<dyn Fn(&str, &str, &str) -> Result<(), String> + Send + Sync>;

/// Process-wide hooks so a fetch handler can start and signal durable tasks
/// owned by the embedding service. Unconfigured processes fail closed.
pub struct DurableControl {
    pub start: StartFn,
    pub send_signal: SignalFn,
}

fn slot() -> &'static RwLock<Option<Arc<DurableControl>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<DurableControl>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

pub fn configure(control: Option<Arc<DurableControl>>) {
    *slot().write().expect("durable control lock") = control;
}

pub fn start_named(name: &str, input_json: &str) -> Result<String, String> {
    let guard = slot().read().map_err(|_| "durable control lock poisoned".to_string())?;
    let Some(control) = guard.as_ref() else {
        return Err("durable scheduler is not configured".into());
    };
    (control.start)(name, input_json)
}

pub fn send_signal(task_id: &str, name: &str, payload_json: &str) -> Result<(), String> {
    let guard = slot().read().map_err(|_| "durable control lock poisoned".to_string())?;
    let Some(control) = guard.as_ref() else {
        return Err("durable scheduler is not configured".into());
    };
    (control.send_signal)(task_id, name, payload_json)
}
