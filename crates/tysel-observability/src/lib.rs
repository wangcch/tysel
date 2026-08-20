//! Structured logs, metrics, traces, and capability spans.
//!
//! JSON logs emit one object per HTTP request and one object per audited
//! capability call. Shared `rid` values correlate the two. Metrics and traces
//! stay out of this crate until later milestones.

use std::io::{self, Write};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

struct JsonLog {
    app: String,
    enabled: bool,
}

static JSON_LOG: RwLock<Option<JsonLog>> = RwLock::new(None);
static REQUEST_IDS: AtomicU64 = AtomicU64::new(1);

/// Allocate a process-local request id for HTTP and capability log lines.
pub fn next_request_id() -> u64 {
    REQUEST_IDS.fetch_add(1, Ordering::Relaxed)
}

/// Replace JSON logging. Tests that never call this stay silent so stderr
/// is not mixed into unit output.
pub fn configure_http_log(app: impl Into<String>, enabled: bool) {
    *JSON_LOG.write().expect("json log lock") = Some(JsonLog { app: app.into(), enabled });
}

/// Current JSON log target. Isolated workers copy this across the Start
/// handshake so denials are recorded in the child process.
pub fn json_log_state() -> (String, bool) {
    match JSON_LOG.read().expect("json log lock").as_ref() {
        Some(config) => (config.app.clone(), config.enabled),
        None => (String::new(), false),
    }
}

pub fn log_http(method: &str, path: &str, status: u16, elapsed: Duration, request_id: u64) {
    let Some(app) = enabled_app() else {
        return;
    };
    write_line(&format_http(&app, method, path, status, elapsed, request_id));
}

/// Record one capability call. SQL, paths, URLs, and secret values stay out.
pub fn log_capability(
    capability: &str,
    operation: &str,
    result: &str,
    elapsed: Duration,
    request_id: u64,
) {
    let Some(app) = enabled_app() else {
        return;
    };
    write_line(&format_capability(&app, capability, operation, result, elapsed, request_id));
}

fn enabled_app() -> Option<String> {
    let guard = JSON_LOG.read().expect("json log lock");
    match guard.as_ref() {
        Some(config) if config.enabled => Some(config.app.clone()),
        _ => None,
    }
}

fn write_line(line: &str) {
    let mut out = io::stderr().lock();
    let _ = writeln!(out, "{line}");
}

pub fn format_http(
    app: &str,
    method: &str,
    path: &str,
    status: u16,
    elapsed: Duration,
    request_id: u64,
) -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let ms = (elapsed.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0;
    let mut value = serde_json::json!({
        "ts": ts,
        "app": app,
        "method": method,
        "path": path,
        "status": status,
        "ms": ms,
    });
    insert_rid(&mut value, request_id);
    value.to_string()
}

pub fn format_capability(
    app: &str,
    capability: &str,
    operation: &str,
    result: &str,
    elapsed: Duration,
    request_id: u64,
) -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let ms = (elapsed.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0;
    let mut value = serde_json::json!({
        "ts": ts,
        "app": app,
        "capability": capability,
        "operation": operation,
        "result": result,
        "ms": ms,
    });
    insert_rid(&mut value, request_id);
    value.to_string()
}

fn insert_rid(value: &mut serde_json::Value, request_id: u64) {
    if request_id != 0 {
        value["rid"] = serde_json::json!(request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn request_line_is_json_without_query_or_headers() {
        let line = format_http("hello-service", "GET", "/hello", 200, Duration::from_millis(12), 0);
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["app"], "hello-service");
        assert_eq!(value["method"], "GET");
        assert_eq!(value["path"], "/hello");
        assert_eq!(value["status"], 200);
        assert!(value.get("headers").is_none());
        assert!(value.get("rid").is_none());
        assert!(!line.contains("token="));
        assert!(!line.contains("Authorization"));
    }

    #[test]
    fn request_line_includes_rid_when_set() {
        let line = format_http("hello-service", "GET", "/hello", 200, Duration::ZERO, 9);
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["rid"], 9);
    }

    #[test]
    fn path_is_json_escaped() {
        let line = format_http("app", "GET", "/quote\"here", 404, Duration::ZERO, 0);
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["path"], "/quote\"here");
        assert_eq!(value["status"], 404);
    }

    #[test]
    fn configure_replaces_the_log_flag() {
        configure_http_log("hello-service", true);
        assert_eq!(json_log_state(), ("hello-service".into(), true));
        configure_http_log("hello-service", false);
        assert_eq!(json_log_state(), ("hello-service".into(), false));
        *super::JSON_LOG.write().expect("json log lock") = None;
        assert_eq!(json_log_state(), (String::new(), false));
    }

    #[test]
    fn capability_line_omits_sql_paths_and_urls() {
        let line = format_capability("hello-service", "postgres", "query", "ok", Duration::ZERO, 4);
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["app"], "hello-service");
        assert_eq!(value["capability"], "postgres");
        assert_eq!(value["operation"], "query");
        assert_eq!(value["result"], "ok");
        assert_eq!(value["rid"], 4);
        assert!(value.get("sql").is_none());
        assert!(value.get("path").is_none());
        assert!(value.get("url").is_none());
    }
}
