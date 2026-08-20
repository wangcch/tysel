//! Supervisor, reactor, and trusted-service data plane.
//!
//! Spike B owns the native HTTP listener. Spike C runs that listener from a
//! runtime stub that memory-maps an embedded TAP trailer.

mod durable;
mod durable_poll;
mod http;
mod service;
mod task_ingress;
mod task_rpc;
#[cfg(unix)]
mod task_service;

pub use durable::{
    DispatchError, DurableDispatcher, DurableRun, DurableRunError, DurableRunStatus,
};
pub use durable_poll::{
    DurablePoller, DurableProgramCatalog, DurableProgramRegistry, PollerError, PollerShutdown,
    ProgramRegistryError,
};
pub use http::{
    AppIsolate, HttpError, SharedPool, bind, bind_with, bind_with_request_limit, handle_stream,
    serve, serve_with_websocket, spawn_app_isolate,
};
pub use service::{StubError, run_stub, run_tap};
pub use task_ingress::{
    CronExpression, TaskIngress, TaskIngressError, TaskRegistry, TriggeredTask,
};
#[cfg(unix)]
pub use task_rpc::{
    TaskModuleWorker, TaskModuleWorkerError, TaskRpcServerError, TaskRpcServerShutdown,
    TaskRpcWorker, TaskRpcWorkerError, serve_task_rpc_unix,
};
pub use task_rpc::{TaskRpcBroker, TaskRpcBrokerError, TaskRpcSession};
#[cfg(unix)]
pub use task_service::{ModuleTaskService, ModuleTaskServiceError};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
