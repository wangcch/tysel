use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _};
use opentelemetry::trace::{Span as _, SpanKind, Tracer as _, TracerProvider as _};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

const INSTRUMENTATION_SCOPE: &str = "tysel-observability";
static OTLP: RwLock<Option<Arc<Otlp>>> = RwLock::new(None);

#[derive(Debug, thiserror::Error)]
pub enum OtlpInitError {
    #[error("OTLP endpoint configuration is invalid")]
    Configuration,
    #[error("failed to configure OTLP trace exporter")]
    Trace,
    #[error("failed to configure OTLP metric exporter")]
    Metrics,
    #[error("failed to shut down OTLP exporter")]
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signals {
    traces: bool,
    metrics: bool,
}

impl Signals {
    fn from_environment() -> Self {
        Self::from_values(std::env::vars())
    }

    fn from_values(values: impl IntoIterator<Item = (String, String)>) -> Self {
        let values: HashMap<_, _> = values.into_iter().collect();
        if values.get("OTEL_SDK_DISABLED").is_some_and(|value| value.eq_ignore_ascii_case("true")) {
            return Self { traces: false, metrics: false };
        }
        let shared = nonempty(&values, "OTEL_EXPORTER_OTLP_ENDPOINT");
        Self {
            traces: shared || nonempty(&values, "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
            metrics: shared || nonempty(&values, "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
        }
    }
}

fn nonempty(values: &HashMap<String, String>, key: &str) -> bool {
    values.get(key).is_some_and(|value| !value.trim().is_empty())
}

struct Otlp {
    app: String,
    tracer_provider: Option<SdkTracerProvider>,
    tracer: Option<SdkTracer>,
    meter_provider: Option<SdkMeterProvider>,
    http_requests: Option<Counter<u64>>,
    http_duration_ms: Option<Histogram<f64>>,
    capability_calls: Option<Counter<u64>>,
    capability_duration_ms: Option<Histogram<f64>>,
}

impl Otlp {
    fn build(app: &str, signals: Signals) -> Result<Self, OtlpInitError> {
        let resource = Resource::builder_empty()
            .with_service_name(super::safe_metadata_label(app))
            .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
            .build();
        let (tracer_provider, tracer) = if signals.traces {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .build()
                .map_err(|_| OtlpInitError::Trace)?;
            let provider = SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(exporter)
                .build();
            let tracer = provider.tracer(INSTRUMENTATION_SCOPE);
            (Some(provider), Some(tracer))
        } else {
            (None, None)
        };
        let (
            meter_provider,
            http_requests,
            http_duration_ms,
            capability_calls,
            capability_duration_ms,
        ) = if signals.metrics {
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .build()
                .map_err(|_| OtlpInitError::Metrics)?;
            let provider = SdkMeterProvider::builder()
                .with_resource(resource)
                .with_periodic_exporter(exporter)
                .build();
            let meter = provider.meter(INSTRUMENTATION_SCOPE);
            (
                Some(provider),
                Some(meter.u64_counter("tysel.http.server.requests").build()),
                Some(meter.f64_histogram("tysel.http.server.duration").with_unit("ms").build()),
                Some(meter.u64_counter("tysel.capability.calls").build()),
                Some(meter.f64_histogram("tysel.capability.duration").with_unit("ms").build()),
            )
        } else {
            (None, None, None, None, None)
        };
        Ok(Self {
            app: super::safe_metadata_label(app),
            tracer_provider,
            tracer,
            meter_provider,
            http_requests,
            http_duration_ms,
            capability_calls,
            capability_duration_ms,
        })
    }

    fn shutdown(&self) -> Result<(), OtlpInitError> {
        if let Some(provider) = &self.tracer_provider {
            provider.shutdown().map_err(|_| OtlpInitError::Shutdown)?;
        }
        if let Some(provider) = &self.meter_provider {
            provider.shutdown().map_err(|_| OtlpInitError::Shutdown)?;
        }
        Ok(())
    }

    fn record_http(&self, method: &str, status: u16, elapsed: Duration, request_id: u64) {
        let method = super::safe_metadata_label(method);
        let status_class = format!("{}xx", status / 100);
        let attributes = vec![
            KeyValue::new("service.name", self.app.clone()),
            KeyValue::new("http.request.method", method),
            KeyValue::new("http.response.status_code", i64::from(status)),
            KeyValue::new("http.response.status_class", status_class),
        ];
        if let Some(counter) = &self.http_requests {
            counter.add(1, &attributes);
        }
        if let Some(histogram) = &self.http_duration_ms {
            histogram.record(elapsed.as_secs_f64() * 1_000.0, &attributes);
        }
        if let Some(tracer) = &self.tracer {
            let ended = SystemTime::now();
            let started = ended.checked_sub(elapsed).unwrap_or(ended);
            let mut span = tracer
                .span_builder("http.server.request")
                .with_kind(SpanKind::Server)
                .with_start_time(started)
                .with_attributes(with_request_id(attributes, request_id))
                .start(tracer);
            span.end_with_timestamp(ended);
        }
    }

    fn record_capability(
        &self,
        capability: &str,
        operation: &str,
        result: &str,
        elapsed: Duration,
        request_id: u64,
    ) {
        let attributes = vec![
            KeyValue::new("service.name", self.app.clone()),
            KeyValue::new("tysel.capability", super::safe_capability_label(capability)),
            KeyValue::new("tysel.operation", super::safe_operation_label(operation)),
            KeyValue::new("tysel.result", super::safe_result_label(result)),
        ];
        if let Some(counter) = &self.capability_calls {
            counter.add(1, &attributes);
        }
        if let Some(histogram) = &self.capability_duration_ms {
            histogram.record(elapsed.as_secs_f64() * 1_000.0, &attributes);
        }
        if let Some(tracer) = &self.tracer {
            let ended = SystemTime::now();
            let started = ended.checked_sub(elapsed).unwrap_or(ended);
            let mut span = tracer
                .span_builder("tysel.capability")
                .with_kind(SpanKind::Internal)
                .with_start_time(started)
                .with_attributes(with_request_id(attributes, request_id))
                .start(tracer);
            span.end_with_timestamp(ended);
        }
    }
}

fn with_request_id(mut attributes: Vec<KeyValue>, request_id: u64) -> Vec<KeyValue> {
    if request_id != 0 {
        attributes.push(KeyValue::new("tysel.request.id", request_id.to_string()));
    }
    attributes
}

/// Process guard that flushes and shuts down exporters on drop.
pub struct OtlpGuard {
    enabled: bool,
}

impl OtlpGuard {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Drop for OtlpGuard {
    fn drop(&mut self) {
        if self.enabled {
            let _ = shutdown_otlp();
        }
    }
}

pub fn configure_otlp(app: &str) -> Result<OtlpGuard, OtlpInitError> {
    let signals = Signals::from_environment();
    if !signals.traces && !signals.metrics {
        return Ok(OtlpGuard { enabled: false });
    }
    for key in [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    ] {
        if let Ok(endpoint) = std::env::var(key)
            && !endpoint.trim().is_empty()
            && !safe_endpoint(&endpoint)
        {
            return Err(OtlpInitError::Configuration);
        }
    }
    let telemetry = Arc::new(Otlp::build(app, signals)?);
    let old = OTLP.write().expect("OTLP lock").replace(telemetry);
    if let Some(old) = old {
        old.shutdown()?;
    }
    Ok(OtlpGuard { enabled: true })
}

fn safe_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > 2_048 || endpoint.contains('?') || endpoint.contains('#') {
        return false;
    }
    let authority_and_path =
        endpoint.strip_prefix("https://").or_else(|| endpoint.strip_prefix("http://"));
    let Some(authority_and_path) = authority_and_path else {
        return false;
    };
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    !authority.is_empty() && !authority.contains('@')
}

pub fn shutdown_otlp() -> Result<(), OtlpInitError> {
    if let Some(telemetry) = OTLP.write().expect("OTLP lock").take() {
        telemetry.shutdown()?;
    }
    Ok(())
}

pub(crate) fn record_http(
    app: &str,
    method: &str,
    status: u16,
    elapsed: Duration,
    request_id: u64,
) {
    if let Some(telemetry) = OTLP.read().expect("OTLP lock").clone()
        && telemetry.app == super::safe_metadata_label(app)
    {
        telemetry.record_http(method, status, elapsed, request_id);
    }
}

pub(crate) fn record_capability(
    app: &str,
    capability: &str,
    operation: &str,
    result: &str,
    elapsed: Duration,
    request_id: u64,
) {
    if let Some(telemetry) = OTLP.read().expect("OTLP lock").clone()
        && telemetry.app == super::safe_metadata_label(app)
    {
        telemetry.record_capability(capability, operation, result, elapsed, request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;

    #[test]
    fn signal_selection_requires_explicit_endpoint_and_honors_disable() {
        let none = Signals::from_values([]);
        assert_eq!(none, Signals { traces: false, metrics: false });
        let shared = Signals::from_values([(
            "OTEL_EXPORTER_OTLP_ENDPOINT".into(),
            "http://collector".into(),
        )]);
        assert_eq!(shared, Signals { traces: true, metrics: true });
        let traces = Signals::from_values([(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(),
            "http://collector/v1/traces".into(),
        )]);
        assert_eq!(traces, Signals { traces: true, metrics: false });
        let disabled = Signals::from_values([
            ("OTEL_EXPORTER_OTLP_ENDPOINT".into(), "http://collector".into()),
            ("OTEL_SDK_DISABLED".into(), "TRUE".into()),
        ]);
        assert_eq!(disabled, Signals { traces: false, metrics: false });
    }

    #[test]
    fn endpoint_validation_rejects_embedded_credentials_and_query_metadata() {
        assert!(safe_endpoint("https://collector.example/v1/traces"));
        assert!(safe_endpoint("http://127.0.0.1:4318"));
        assert!(!safe_endpoint("collector.example"));
        assert!(!safe_endpoint("https://user:secret@collector.example"));
        assert!(!safe_endpoint("https://collector.example?token=secret"));
    }

    #[test]
    fn telemetry_labels_fail_closed_instead_of_exporting_sensitive_metadata() {
        assert_eq!(super::super::safe_metadata_label("postgres"), "postgres");
        assert_eq!(super::super::safe_metadata_label("tysel:fs"), "tysel:fs");
        assert_eq!(super::super::safe_metadata_label("https://secret"), "redacted");
        assert_eq!(super::super::safe_metadata_label("query?token=secret"), "redacted");
        assert_eq!(super::super::safe_metadata_label("Bearer secret"), "redacted");
        assert_eq!(
            super::super::safe_metadata_label(
                &"x".repeat(super::super::MAX_METADATA_LABEL_BYTES + 1)
            ),
            "redacted"
        );
    }

    #[test]
    fn otlp_http_exports_expected_signals_without_sensitive_metadata() {
        if std::env::var_os("TYSEL_OTLP_CHILD").is_some() {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind collector");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "otlp::tests::otlp_child_export", "--nocapture"])
            .env("TYSEL_OTLP_CHILD", "1")
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
            .spawn()
            .expect("spawn OTLP producer");

        let mut bodies = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept OTLP export");
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut chunk).expect("read OTLP headers");
                assert!(read > 0, "OTLP request closed before headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..header_end]).expect("HTTP headers");
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .expect("content-length header");
            while request.len() - header_end < content_length {
                let read = stream.read(&mut chunk).expect("read OTLP body");
                assert!(read > 0, "OTLP request closed before body");
                request.extend_from_slice(&chunk[..read]);
            }
            bodies.extend_from_slice(&request[header_end..header_end + content_length]);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .expect("reply to exporter");
        }
        assert!(child.wait().expect("wait for producer").success());
        let payload = String::from_utf8_lossy(&bodies);
        assert!(payload.contains("http.server.request"));
        assert!(payload.contains("tysel.capability"));
        for sensitive in
            ["/tenant/secret", "SELECT password", "https://secret.example", "Bearer secret-token"]
        {
            assert!(!payload.contains(sensitive), "OTLP leaked {sensitive:?}");
        }
    }

    #[test]
    fn otlp_child_export() {
        if std::env::var_os("TYSEL_OTLP_CHILD").is_none() {
            return;
        }
        super::super::configure_http_log("otel-test", false);
        let guard = configure_otlp("otel-test").expect("configure OTLP");
        assert!(guard.is_enabled());
        super::super::log_http(
            "GET",
            "/tenant/secret?token=secret",
            200,
            Duration::from_millis(2),
            7,
        );
        super::super::log_capability(
            "https://secret.example",
            "SELECT password",
            "Bearer secret-token",
            Duration::from_millis(1),
            7,
        );
        drop(guard);
    }
}
