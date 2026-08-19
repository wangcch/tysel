//! Structured logs, metrics, traces, and capability spans.
//!
//! M1 emits one JSON object per HTTP request on stderr. Metrics and traces
//! stay out of this crate until later milestones.

use std::io::{self, Write};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

struct HttpLog {
    app: String,
    enabled: bool,
}

static HTTP_LOG: RwLock<Option<HttpLog>> = RwLock::new(None);

/// Replace request logging. Tests that never call this stay silent so stderr
/// is not mixed into unit output.
pub fn configure_http_log(app: impl Into<String>, enabled: bool) {
    *HTTP_LOG.write().expect("http log lock") = Some(HttpLog { app: app.into(), enabled });
}

pub fn log_http(method: &str, path: &str, status: u16, elapsed: Duration) {
    let (app, enabled) = {
        let guard = HTTP_LOG.read().expect("http log lock");
        match guard.as_ref() {
            Some(config) => (config.app.clone(), config.enabled),
            None => return,
        }
    };
    if !enabled {
        return;
    }
    let line = format_http(&app, method, path, status, elapsed);
    let mut out = io::stderr().lock();
    let _ = writeln!(out, "{line}");
}

pub fn format_http(app: &str, method: &str, path: &str, status: u16, elapsed: Duration) -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let ms = (elapsed.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0;
    serde_json::json!({
        "ts": ts,
        "app": app,
        "method": method,
        "path": path,
        "status": status,
        "ms": ms,
    })
    .to_string()
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
        let line = format_http("hello-service", "GET", "/hello", 200, Duration::from_millis(12));
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["app"], "hello-service");
        assert_eq!(value["method"], "GET");
        assert_eq!(value["path"], "/hello");
        assert_eq!(value["status"], 200);
        assert!(value.get("headers").is_none());
        assert!(!line.contains("token="));
        assert!(!line.contains("Authorization"));
    }

    #[test]
    fn path_is_json_escaped() {
        let line = format_http("app", "GET", "/quote\"here", 404, Duration::ZERO);
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["path"], "/quote\"here");
        assert_eq!(value["status"], 404);
    }

    #[test]
    fn configure_replaces_the_log_flag() {
        configure_http_log("hello-service", true);
        assert_eq!(enabled(), Some(true));
        configure_http_log("hello-service", false);
        assert_eq!(enabled(), Some(false));
        *super::HTTP_LOG.write().expect("http log lock") = None;
        assert_eq!(enabled(), None);
    }

    fn enabled() -> Option<bool> {
        super::HTTP_LOG.read().expect("http log lock").as_ref().map(|config| config.enabled)
    }
}
