//! QuickJS-ng compatibility engine.
//!
//! Spike A pins a runtime to one worker thread and settles promises through a
//! completion queue. Spike B reuses that isolate as a fetch-handler pool behind
//! a native HTTP listener.

use std::path::Path;

use serde::Deserialize;

mod control;
mod cpu;
mod durable;
mod fetch;
mod fetch_policy;
mod host;
mod isolate;
mod llm;
mod pool;
mod queue;
mod secrets;
mod task_module;
mod trust;

pub use control::{DurableControl, configure as configure_durable_control};
pub use durable::DurableSession;
pub use fetch_policy::configure as configure_fetch_hosts;
pub use isolate::{
    IsolateCancel, encode_durable_export, eval, eval_cancellable, eval_durable,
    eval_durable_module, eval_with_reactor, eval_with_reactor_deadline,
};
pub use llm::configure as configure_llm;
pub use pool::{IncomingHttp, IsolatePool, OutgoingHttpBody};
pub use queue::{IoCompletion, IoRequest, IoWork, OpId, Reactor, STREAM_WINDOW, open_bridge};
pub use secrets::{
    configure as configure_secrets, load_declared, parse_dotenv, resolve as resolve_secret,
};
pub use task_module::{
    ModuleMetadata, ModuleTaskDefinition, ModuleTaskKind, inspect_durable_exports,
    inspect_task_module, invoke_task_module,
};
pub use trust::configure as configure_policy;

/// Versioned identity of the QuickJS adapter used by this compatibility
/// engine. This changes when the Rust adapter or underlying engine family
/// changes in a way that requires runtime conformance to be re-established.
pub const QUICKJS_ENGINE_VERSION: &str = env!("TYSEL_QUICKJS_ENGINE_VERSION");
pub const QUICKJS_ADAPTER_ID: &str = env!("TYSEL_QUICKJS_ADAPTER_ID");

/// Machine-readable compatibility contract embedded into the engine binary.
pub const RUNTIME_COMPATIBILITY_JSON: &str = include_str!("../../../runtime-js/compatibility.json");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TapCompatibility {
    pub minimum_supported_version: u32,
    pub maximum_supported_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebApiCompatibilityIdentity {
    pub profile: String,
    pub compatibility_schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuickJsComponentIdentity {
    pub name: String,
    pub version: String,
    pub repository: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuickJsProvenance {
    pub release_status: String,
    pub allowed_release_channels: Vec<String>,
    pub adapter: QuickJsComponentIdentity,
    pub engine: QuickJsComponentIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCompatibility {
    pub schema_version: u32,
    pub runtime_js_version: String,
    pub tap: TapCompatibility,
    pub component_abi_version: String,
    pub quickjs_adapter: String,
    pub quickjs: QuickJsProvenance,
    pub web_api: WebApiCompatibilityIdentity,
}

/// Parse the compatibility contract embedded into this engine build.
pub fn runtime_compatibility() -> Result<RuntimeCompatibility, serde_json::Error> {
    serde_json::from_str(RUNTIME_COMPATIBILITY_JSON)
}

/// Apply the TAP execution profile. `isolated` denies fetch, SQLite,
/// WebSocket, Postgres, Redis, and filesystem access; every other profile is the
/// trusted service path.
pub fn configure_execution_profile(profile: &str) {
    trust::configure(tysel_policy::Policy::from_profile(profile));
}

/// Pin the process-wide SQLite file used by trusted-path `tysel.sqlite`.
///
/// An empty path keeps the default in-memory database. Relative paths are
/// resolved against `root` when provided. The first successful call wins.
pub fn configure_sqlite_path(path: &str, root: Option<&Path>) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    if path == ":memory:" || Path::new(path).is_absolute() {
        tysel_cap_sqlite::configure_path(path);
        return;
    }
    let resolved = match root {
        Some(root) => root.join(path).to_string_lossy().into_owned(),
        None => path.to_owned(),
    };
    tysel_cap_sqlite::configure_path(resolved);
}

/// Pin trusted-path filesystem roots. Relative paths are resolved against
/// `root` when provided. Unconfigured processes deny every path.
pub fn configure_fs(read: Vec<String>, write: Vec<String>, root: Option<&Path>) {
    tysel_cap_fs::configure(read, write, root);
}

/// Pin the trusted-path Postgres URL resolved from `TYSEL_POSTGRES_<NAME>`.
/// `None` leaves Postgres unconfigured.
pub fn configure_postgres(url: Option<String>, read_only: bool) {
    tysel_cap_postgres::configure(url, read_only);
}

/// Pin the trusted-path Redis URL resolved from `TYSEL_REDIS_<NAME>`.
/// `None` leaves Redis unconfigured.
pub fn configure_redis(url: Option<String>, read_only: bool) {
    tysel_cap_redis::configure(url, read_only);
}

#[cfg(test)]
mod tests;
