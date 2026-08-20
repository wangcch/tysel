//! Policy-bound LLM provider gateway with opaque credentials and bounded I/O.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Semaphore, watch};

use bytes::Bytes;
use http_body_util::Full;
use hyper::Request;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

pub const MAX_LLM_MODEL_BYTES: usize = 128;
pub const MAX_LLM_REQUEST_ID_BYTES: usize = 128;
pub const MAX_LLM_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_LLM_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_LLM_ERROR_BYTES: usize = 4 * 1024;
pub const MAX_LLM_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmRequest {
    pub request_id: String,
    pub model: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmResponse {
    pub output: Value,
    #[serde(default)]
    pub usage: LlmUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Resolved credential whose Debug output is always redacted. Only provider
/// implementations can deliberately expose it at the outbound request edge.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, LlmError> {
        let value = value.into();
        if value.is_empty() {
            return Err(LlmError::SecretUnavailable);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

pub trait SecretResolver: Send + Sync {
    fn resolve(&self, handle: &str) -> Option<SecretValue>;
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub request: LlmRequest,
    pub credential: Option<SecretValue>,
}

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LlmResponse, LlmProviderError>> + Send + 'a>>;

pub trait LlmProvider: Send + Sync {
    fn generate<'a>(&'a self, request: ProviderRequest) -> ProviderFuture<'a>;
}

/// Native provider for OpenAI-compatible `POST` endpoints. The endpoint is an
/// exact URL (for example `https://api.openai.com/v1/responses`); an optional
/// upstream model maps an application alias such as `default` to a provider
/// model without exposing that choice to application code.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    endpoint: hyper::Uri,
    upstream_model: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(endpoint: &str, upstream_model: Option<String>) -> Result<Self, LlmError> {
        let endpoint: hyper::Uri = endpoint
            .parse()
            .map_err(|_| LlmError::InvalidConfiguration("invalid LLM endpoint".into()))?;
        if !matches!(endpoint.scheme_str(), Some("http" | "https")) || endpoint.host().is_none() {
            return Err(LlmError::InvalidConfiguration(
                "LLM endpoint must be an absolute http(s) URL".into(),
            ));
        }
        if let Some(model) = &upstream_model {
            validate_identifier("upstream model", model, MAX_LLM_MODEL_BYTES)?;
        }
        Ok(Self { endpoint, upstream_model })
    }

    async fn send(&self, request: ProviderRequest) -> Result<LlmResponse, LlmProviderError> {
        let credential = request
            .credential
            .as_ref()
            .ok_or_else(|| LlmProviderError::new("provider credential is missing", false))?;
        let model = self.upstream_model.as_deref().unwrap_or(&request.request.model);
        let mut body = serde_json::json!({
            "model": model,
            "input": request.request.input,
        });
        let body_object = body.as_object_mut().expect("provider request object");
        if let Some(system) = request.request.system {
            body_object.insert("instructions".into(), Value::String(system));
        }
        if let Some(tokens) = request.request.max_output_tokens {
            body_object.insert("max_output_tokens".into(), Value::from(tokens));
        }
        if let Some(temperature) = request.request.temperature {
            body_object.insert("temperature".into(), Value::from(temperature));
        }
        let body = serde_json::to_vec(&body)
            .map_err(|error| LlmProviderError::new(error.to_string(), false))?;
        let response = send_http_json(&self.endpoint, credential.expose(), body).await?;
        let status = response.status();
        let bytes = read_bounded_body(response.into_body()).await?;
        if !status.is_success() {
            let message = String::from_utf8_lossy(&bytes);
            return Err(LlmProviderError::new(
                format!("provider returned HTTP {}: {message}", status.as_u16()),
                status.as_u16() == 429 || status.is_server_error(),
            ));
        }
        parse_openai_response(&bytes)
    }
}

impl LlmProvider for OpenAiCompatibleProvider {
    fn generate<'a>(&'a self, request: ProviderRequest) -> ProviderFuture<'a> {
        Box::pin(self.send(request))
    }
}

async fn send_http_json(
    uri: &hyper::Uri,
    credential: &str,
    body: Vec<u8>,
) -> Result<hyper::Response<Incoming>, LlmProviderError> {
    let https = uri.scheme_str() == Some("https");
    let host = uri.host().expect("validated LLM host").to_owned();
    let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
    let stream = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|error| LlmProviderError::new(error.to_string(), true))?;
    let path = uri.path_and_query().map_or("/", |path| path.as_str()).to_owned();
    let host_header = if (!https && port == 80) || (https && port == 443) {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(hyper::header::HOST, host_header)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::AUTHORIZATION, format!("Bearer {credential}"))
        .body(Full::new(Bytes::from(body)))
        .map_err(|error| LlmProviderError::new(error.to_string(), false))?;
    if https {
        let connector = tokio_native_tls::TlsConnector::from(
            native_tls::TlsConnector::new()
                .map_err(|error| LlmProviderError::new(error.to_string(), true))?,
        );
        let stream = connector
            .connect(&host, stream)
            .await
            .map_err(|error| LlmProviderError::new(error.to_string(), true))?;
        send_request(TokioIo::new(stream), request).await
    } else {
        send_request(TokioIo::new(stream), request).await
    }
}

async fn send_request<I>(
    io: I,
    request: Request<Full<Bytes>>,
) -> Result<hyper::Response<Incoming>, LlmProviderError>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|error| LlmProviderError::new(error.to_string(), true))?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
        .send_request(request)
        .await
        .map_err(|error| LlmProviderError::new(error.to_string(), true))
}

async fn read_bounded_body(mut body: Incoming) -> Result<Vec<u8>, LlmProviderError> {
    use http_body_util::BodyExt;

    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| LlmProviderError::new(error.to_string(), true))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        if bytes.len().saturating_add(chunk.len()) > MAX_LLM_OUTPUT_BYTES {
            return Err(LlmProviderError::new("provider response exceeds limit", false));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_openai_response(bytes: &[u8]) -> Result<LlmResponse, LlmProviderError> {
    let response: Value = serde_json::from_slice(bytes)
        .map_err(|error| LlmProviderError::new(format!("invalid provider JSON: {error}"), false))?;
    let output = response
        .get("output_text")
        .cloned()
        .or_else(|| extract_output_text(&response).map(Value::String))
        .or_else(|| response.get("output").cloned())
        .ok_or_else(|| LlmProviderError::new("provider response has no output", false))?;
    let usage = response.get("usage");
    Ok(LlmResponse {
        output,
        usage: LlmUsage {
            input_tokens: usage
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output_tokens: usage
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        provider_request_id: response.get("id").and_then(Value::as_str).map(str::to_owned),
    })
}

fn extract_output_text(response: &Value) -> Option<String> {
    let mut output = String::new();
    for item in response.get("output")?.as_array()? {
        for content in item.get("content").and_then(Value::as_array).into_iter().flatten() {
            if content.get("type").and_then(Value::as_str) == Some("output_text") {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    output.push_str(text);
                }
            }
        }
    }
    (!output.is_empty()).then_some(output)
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct LlmProviderError {
    message: String,
    pub retryable: bool,
}

impl LlmProviderError {
    pub fn new(message: impl Into<String>, retryable: bool) -> Self {
        let mut message = message.into();
        truncate_utf8(&mut message, MAX_LLM_ERROR_BYTES);
        Self { message, retryable }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone)]
pub struct LlmRoute {
    pub provider_name: String,
    pub provider: Arc<dyn LlmProvider>,
    pub credential_handle: Option<String>,
}

impl fmt::Debug for LlmRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LlmRoute")
            .field("provider_name", &self.provider_name)
            .field("credential_handle", &self.credential_handle.as_ref().map(|_| "[OPAQUE]"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LlmGatewayConfig {
    pub timeout_ms: u64,
    pub max_concurrent: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for LlmGatewayConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 60_000,
            max_concurrent: 16,
            max_input_bytes: MAX_LLM_INPUT_BYTES,
            max_output_bytes: MAX_LLM_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmAuditEvent {
    pub request_id: String,
    pub model: String,
    pub provider: String,
    pub input_bytes: usize,
    pub output_bytes: Option<usize>,
    pub elapsed_ms: u64,
    pub outcome: LlmAuditOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmAuditOutcome {
    Completed,
    Rejected,
    ProviderFailed,
    TimedOut,
    Canceled,
}

pub trait LlmAuditSink: Send + Sync {
    fn record(&self, event: LlmAuditEvent);
}

#[derive(Debug, Default)]
pub struct NoopAudit;

impl LlmAuditSink for NoopAudit {
    fn record(&self, _event: LlmAuditEvent) {}
}

#[derive(Clone, Debug)]
pub struct LlmCancel {
    sender: watch::Sender<bool>,
}

impl Default for LlmCancel {
    fn default() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }
}

impl LlmCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }
}

pub struct LlmGateway {
    routes: BTreeMap<String, LlmRoute>,
    secrets: Arc<dyn SecretResolver>,
    audit: Arc<dyn LlmAuditSink>,
    permits: Semaphore,
    config: LlmGatewayConfig,
}

impl LlmGateway {
    pub fn new(
        routes: BTreeMap<String, LlmRoute>,
        secrets: Arc<dyn SecretResolver>,
        audit: Arc<dyn LlmAuditSink>,
        config: LlmGatewayConfig,
    ) -> Result<Self, LlmError> {
        validate_config(config)?;
        if routes.is_empty() {
            return Err(LlmError::NoRoutes);
        }
        for (model, route) in &routes {
            validate_identifier("model", model, MAX_LLM_MODEL_BYTES)?;
            validate_identifier("provider", &route.provider_name, MAX_LLM_MODEL_BYTES)?;
            if route.credential_handle.as_ref().is_some_and(String::is_empty) {
                return Err(LlmError::InvalidConfiguration("empty credential handle".into()));
            }
        }
        Ok(Self { routes, secrets, audit, permits: Semaphore::new(config.max_concurrent), config })
    }

    pub async fn generate(
        &self,
        request: LlmRequest,
        cancel: &LlmCancel,
    ) -> Result<LlmResponse, LlmError> {
        let started = Instant::now();
        let route = match self.routes.get(&request.model) {
            Some(route) => route.clone(),
            None => {
                self.audit_rejected(&request, "<denied>", 0, started);
                return Err(LlmError::ModelDenied);
            }
        };
        let input_bytes = match validate_request(&request, self.config.max_input_bytes) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.audit_rejected(&request, &route.provider_name, 0, started);
                return Err(error);
            }
        };
        if cancel.is_cancelled() {
            self.audit(&request, &route, input_bytes, None, started, LlmAuditOutcome::Canceled);
            return Err(LlmError::Canceled);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(self.config.timeout_ms);
        let permit = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                self.audit(&request, &route, input_bytes, None, started, LlmAuditOutcome::Canceled);
                return Err(LlmError::Canceled);
            }
            result = tokio::time::timeout_at(deadline, self.permits.acquire()) => {
                match result {
                    Ok(Ok(permit)) => permit,
                    Ok(Err(_)) => {
                        self.audit(&request, &route, input_bytes, None, started, LlmAuditOutcome::Rejected);
                        return Err(LlmError::Closed);
                    }
                    Err(_) => {
                        self.audit(&request, &route, input_bytes, None, started, LlmAuditOutcome::TimedOut);
                        return Err(LlmError::TimedOut);
                    }
                }
            }
        };
        let credential = match &route.credential_handle {
            Some(handle) => match self.secrets.resolve(handle) {
                Some(secret) => Some(secret),
                None => {
                    self.audit(
                        &request,
                        &route,
                        input_bytes,
                        None,
                        started,
                        LlmAuditOutcome::Rejected,
                    );
                    return Err(LlmError::SecretUnavailable);
                }
            },
            None => None,
        };
        let provider_request = ProviderRequest { request: request.clone(), credential };
        let generated = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(LlmError::Canceled),
            result = tokio::time::timeout_at(deadline, route.provider.generate(provider_request)) => {
                match result {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(error)) => Err(LlmError::Provider(error)),
                    Err(_) => Err(LlmError::TimedOut),
                }
            }
        };
        drop(permit);
        match generated {
            Ok(response) => {
                if response
                    .provider_request_id
                    .as_ref()
                    .is_some_and(|id| !valid_identifier(id, MAX_LLM_REQUEST_ID_BYTES))
                {
                    self.audit(
                        &request,
                        &route,
                        input_bytes,
                        None,
                        started,
                        LlmAuditOutcome::Rejected,
                    );
                    return Err(LlmError::InvalidProviderResponse);
                }
                let output_bytes = serde_json::to_vec(&response)?.len();
                if output_bytes > self.config.max_output_bytes {
                    self.audit(
                        &request,
                        &route,
                        input_bytes,
                        Some(output_bytes),
                        started,
                        LlmAuditOutcome::Rejected,
                    );
                    return Err(LlmError::OutputTooLarge(output_bytes));
                }
                self.audit(
                    &request,
                    &route,
                    input_bytes,
                    Some(output_bytes),
                    started,
                    LlmAuditOutcome::Completed,
                );
                Ok(response)
            }
            Err(error) => {
                let outcome = match error {
                    LlmError::Canceled => LlmAuditOutcome::Canceled,
                    LlmError::TimedOut => LlmAuditOutcome::TimedOut,
                    _ => LlmAuditOutcome::ProviderFailed,
                };
                self.audit(&request, &route, input_bytes, None, started, outcome);
                Err(error)
            }
        }
    }

    fn audit(
        &self,
        request: &LlmRequest,
        route: &LlmRoute,
        input_bytes: usize,
        output_bytes: Option<usize>,
        started: Instant,
        outcome: LlmAuditOutcome,
    ) {
        self.audit.record(LlmAuditEvent {
            request_id: request.request_id.clone(),
            model: request.model.clone(),
            provider: route.provider_name.clone(),
            input_bytes,
            output_bytes,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            outcome,
        });
    }

    fn audit_rejected(
        &self,
        request: &LlmRequest,
        provider: &str,
        input_bytes: usize,
        started: Instant,
    ) {
        self.audit.record(LlmAuditEvent {
            request_id: request.request_id.clone(),
            model: request.model.clone(),
            provider: provider.into(),
            input_bytes,
            output_bytes: None,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            outcome: LlmAuditOutcome::Rejected,
        });
    }
}

fn validate_config(config: LlmGatewayConfig) -> Result<(), LlmError> {
    if config.timeout_ms == 0 || config.timeout_ms > MAX_LLM_TIMEOUT_MS {
        return Err(LlmError::InvalidConfiguration("invalid timeout".into()));
    }
    if config.max_concurrent == 0
        || config.max_input_bytes == 0
        || config.max_input_bytes > MAX_LLM_INPUT_BYTES
        || config.max_output_bytes == 0
        || config.max_output_bytes > MAX_LLM_OUTPUT_BYTES
    {
        return Err(LlmError::InvalidConfiguration("invalid gateway bounds".into()));
    }
    Ok(())
}

fn validate_request(request: &LlmRequest, maximum: usize) -> Result<usize, LlmError> {
    if !valid_identifier(&request.request_id, MAX_LLM_REQUEST_ID_BYTES) {
        return Err(LlmError::InvalidRequest("invalid request id".into()));
    }
    if !valid_identifier(&request.model, MAX_LLM_MODEL_BYTES) {
        return Err(LlmError::InvalidRequest("invalid model".into()));
    }
    if request.temperature.is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(LlmError::InvalidRequest("temperature must be between 0 and 2".into()));
    }
    if request.max_output_tokens == Some(0) {
        return Err(LlmError::InvalidRequest("max_output_tokens must be positive".into()));
    }
    let bytes = serde_json::to_vec(request)?.len();
    if bytes > maximum {
        return Err(LlmError::InputTooLarge(bytes));
    }
    Ok(bytes)
}

fn validate_identifier(label: &str, value: &str, maximum: usize) -> Result<(), LlmError> {
    if !valid_identifier(value, maximum) {
        return Err(LlmError::InvalidConfiguration(format!("invalid {label}")));
    }
    Ok(())
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._:/".contains(&byte))
}

fn truncate_utf8(value: &mut String, maximum: usize) {
    if value.len() <= maximum {
        return;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no LLM model routes are configured")]
    NoRoutes,
    #[error("LLM model is not allowed")]
    ModelDenied,
    #[error("LLM credential is unavailable")]
    SecretUnavailable,
    #[error("LLM request was canceled")]
    Canceled,
    #[error("LLM request timed out")]
    TimedOut,
    #[error("LLM gateway is closed")]
    Closed,
    #[error("LLM input is {0} bytes; configured maximum was exceeded")]
    InputTooLarge(usize),
    #[error("LLM output is {0} bytes; configured maximum was exceeded")]
    OutputTooLarge(usize),
    #[error("invalid LLM configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid LLM request: {0}")]
    InvalidRequest(String),
    #[error("LLM provider returned an invalid response")]
    InvalidProviderResponse,
    #[error("LLM provider: {0}")]
    Provider(#[from] LlmProviderError),
    #[error("LLM JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Secrets;

    impl SecretResolver for Secrets {
        fn resolve(&self, handle: &str) -> Option<SecretValue> {
            (handle == "secret://provider").then(|| SecretValue::new("raw-api-key").unwrap())
        }
    }

    #[derive(Default)]
    struct Audit(Mutex<Vec<LlmAuditEvent>>);

    impl LlmAuditSink for Audit {
        fn record(&self, event: LlmAuditEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    struct Provider {
        delay: Duration,
        seen_secret: Arc<Mutex<Option<String>>>,
        output: Value,
    }

    impl LlmProvider for Provider {
        fn generate<'a>(&'a self, request: ProviderRequest) -> ProviderFuture<'a> {
            Box::pin(async move {
                if !self.delay.is_zero() {
                    tokio::time::sleep(self.delay).await;
                }
                *self.seen_secret.lock().unwrap() =
                    request.credential.map(|secret| secret.expose().to_owned());
                Ok(LlmResponse {
                    output: self.output.clone(),
                    usage: LlmUsage { input_tokens: 3, output_tokens: 2 },
                    provider_request_id: Some("provider-1".into()),
                })
            })
        }
    }

    fn request() -> LlmRequest {
        LlmRequest {
            request_id: "request-1".into(),
            model: "default".into(),
            input: serde_json::json!({ "customer": 7 }),
            system: None,
            max_output_tokens: Some(100),
            temperature: Some(0.2),
        }
    }

    fn test_gateway(
        delay: Duration,
        timeout_ms: u64,
    ) -> (LlmGateway, Arc<Audit>, Arc<Mutex<Option<String>>>) {
        let seen_secret = Arc::new(Mutex::new(None));
        let provider = Arc::new(Provider {
            delay,
            seen_secret: Arc::clone(&seen_secret),
            output: serde_json::json!({ "analysis": "safe" }),
        });
        let audit = Arc::new(Audit::default());
        let gateway = LlmGateway::new(
            BTreeMap::from([(
                "default".into(),
                LlmRoute {
                    provider_name: "fake".into(),
                    provider,
                    credential_handle: Some("secret://provider".into()),
                },
            )]),
            Arc::new(Secrets),
            audit.clone(),
            LlmGatewayConfig { timeout_ms, ..LlmGatewayConfig::default() },
        )
        .unwrap();
        (gateway, audit, seen_secret)
    }

    #[tokio::test]
    async fn resolves_secret_only_at_provider_and_records_metadata_only() {
        let (gateway, audit, secret) = test_gateway(Duration::ZERO, 1_000);
        let response = gateway.generate(request(), &LlmCancel::new()).await.unwrap();
        assert_eq!(response.output["analysis"], "safe");
        assert_eq!(secret.lock().unwrap().as_deref(), Some("raw-api-key"));
        assert_eq!(
            format!("{:?}", SecretValue::new("raw-api-key").unwrap()),
            "SecretValue([REDACTED])"
        );
        let events = audit.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, LlmAuditOutcome::Completed);
        assert_eq!(events[0].provider, "fake");
    }

    #[tokio::test]
    async fn timeout_and_cancel_drop_provider_work_and_are_audited() {
        let (gateway, audit, _) = test_gateway(Duration::from_millis(100), 10);
        assert!(matches!(
            gateway.generate(request(), &LlmCancel::new()).await,
            Err(LlmError::TimedOut)
        ));
        assert_eq!(audit.0.lock().unwrap()[0].outcome, LlmAuditOutcome::TimedOut);

        let (gateway, audit, _) = test_gateway(Duration::from_millis(100), 1_000);
        let cancel = LlmCancel::new();
        let cancel_task = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            cancel_task.cancel();
        });
        assert!(matches!(gateway.generate(request(), &cancel).await, Err(LlmError::Canceled)));
        assert_eq!(audit.0.lock().unwrap()[0].outcome, LlmAuditOutcome::Canceled);
    }

    #[tokio::test]
    async fn denies_unknown_models_and_invalid_or_oversized_requests() {
        let (gateway, audit, _) = test_gateway(Duration::ZERO, 1_000);
        let mut denied = request();
        denied.model = "unconfigured".into();
        assert!(matches!(
            gateway.generate(denied, &LlmCancel::new()).await,
            Err(LlmError::ModelDenied)
        ));
        let mut invalid = request();
        invalid.temperature = Some(f64::NAN);
        assert!(matches!(
            gateway.generate(invalid, &LlmCancel::new()).await,
            Err(LlmError::InvalidRequest(_))
        ));
        let events = audit.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.outcome == LlmAuditOutcome::Rejected));
    }

    #[test]
    fn parses_openai_responses_output_text_and_usage() {
        let response = parse_openai_response(
            br#"{
              "id":"response-1",
              "output":[{"content":[{"type":"output_text","text":"hello "},{"type":"output_text","text":"world"}]}],
              "usage":{"input_tokens":4,"output_tokens":2}
            }"#,
        )
        .unwrap();
        assert_eq!(response.output, "hello world");
        assert_eq!(response.usage, LlmUsage { input_tokens: 4, output_tokens: 2 });
        assert_eq!(response.provider_request_id.as_deref(), Some("response-1"));
    }

    #[test]
    fn rejects_invalid_routes_and_bounds_provider_errors() {
        let error = LlmProviderError::new("界".repeat(MAX_LLM_ERROR_BYTES), true);
        assert!(error.message().len() <= MAX_LLM_ERROR_BYTES);
        assert!(
            LlmGateway::new(
                BTreeMap::new(),
                Arc::new(Secrets),
                Arc::new(NoopAudit),
                LlmGatewayConfig::default()
            )
            .is_err()
        );
    }

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }
}
