//! Execution engine abstraction.
//!
//! v0.x implements QuickJS-ng (`tysel-engine-qjs`). Wasm Component and Static
//! AOT engines plug in behind this trait later.

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsolateId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandlerId(pub u64);

#[derive(Debug, Clone)]
pub struct IsolateConfig {
    pub memory_limit_bytes: usize,
    pub cpu_ms_per_turn: u64,
    pub request_timeout_ms: u64,
}

impl Default for IsolateConfig {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 32 * 1024 * 1024,
            cpu_ms_per_turn: 50,
            request_timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptReason {
    Timeout,
    MemoryLimit,
    Cancelled,
    HostShutdown,
}

/// Neutral value allowed across isolate, process, and capability boundaries.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Record(Vec<(String, Value)>),
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("isolate error: {0}")]
    Isolate(String),
    #[error("module error: {0}")]
    Module(String),
    #[error("interrupted: {0:?}")]
    Interrupted(InterruptReason),
}

/// JavaScript / Wasm / AOT execution backend.
///
/// `invoke` stays out of this trait until Spike A lands a completion queue.
pub trait ExecutionEngine: Send + Sync {
    fn create_isolate(&self, config: IsolateConfig) -> Result<IsolateId, EngineError>;
    fn load_module(&self, isolate: IsolateId, bundle: &[u8]) -> Result<ModuleId, EngineError>;
    fn interrupt(&self, isolate: IsolateId, reason: InterruptReason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_isolate_budget() {
        let config = IsolateConfig::default();
        assert_eq!(config.cpu_ms_per_turn, 50);
        assert!(config.memory_limit_bytes > 0);
    }
}
