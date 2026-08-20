use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::Deserialize;
use tysel_cap_llm::{LlmCancel, LlmGateway, LlmRequest};
use tysel_engine::{InterruptReason, Value};

use crate::queue::OpId;

static GATEWAY: RwLock<Option<Arc<LlmGateway>>> = RwLock::new(None);

pub fn configure(gateway: Option<Arc<LlmGateway>>) {
    *GATEWAY.write().expect("LLM gateway lock") = gateway;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JsLlmRequest {
    model: String,
    input: serde_json::Value,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f64>,
}

pub async fn generate(
    request_json: String,
    isolate_request_id: u64,
    op_id: OpId,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Value, String> {
    let gateway = GATEWAY
        .read()
        .expect("LLM gateway lock")
        .clone()
        .ok_or_else(|| "LLM capability is not configured".to_string())?;
    let request: JsLlmRequest = serde_json::from_str(&request_json)
        .map_err(|error| format!("invalid LLM request: {error}"))?;
    let request = LlmRequest {
        request_id: format!("qjs-{isolate_request_id}-{}", op_id.0),
        model: request.model,
        input: request.input,
        system: request.system,
        max_output_tokens: request.max_output_tokens,
        temperature: request.temperature,
    };
    let llm_cancel = LlmCancel::new();
    let generated = gateway.generate(request, &llm_cancel);
    tokio::pin!(generated);
    let response = tokio::select! {
        biased;
        result = &mut generated => result,
        _ = crate::queue::cancelled(&cancel, deadline) => {
            llm_cancel.cancel();
            generated.await
        }
    }
    .map_err(|error| match error {
        tysel_cap_llm::LlmError::Canceled => {
            format!("interrupted: {:?}", InterruptReason::Cancelled)
        }
        tysel_cap_llm::LlmError::TimedOut => {
            format!("interrupted: {:?}", InterruptReason::Timeout)
        }
        error => error.to_string(),
    })?;
    let value = serde_json::to_value(response).map_err(|error| error.to_string())?;
    Ok(crate::isolate::from_json(value))
}
