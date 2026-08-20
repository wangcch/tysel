use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tysel_cap_llm::{
    LlmAuditEvent, LlmAuditSink, LlmGateway, LlmGatewayConfig, LlmRoute, OpenAiCompatibleProvider,
    SecretResolver, SecretValue,
};
use tysel_engine::IsolateConfig;
use tysel_engine_wasm::{
    AotComponentRef, ComponentEngineConfig, MAX_COMPONENT_EXECUTION_MS, MAX_COMPONENT_INPUT_BYTES,
    MAX_COMPONENT_MEMORY_BYTES, WasmComponentEngine,
};
use tysel_package::Tap;

use crate::http::{AppIsolate, HttpError, serve_with_websocket, spawn_app_isolate};
#[cfg(unix)]
use crate::{ModuleTaskService, ModuleTaskServiceError};

#[derive(Debug, thiserror::Error)]
pub enum StubError {
    #[error(transparent)]
    Package(#[from] tysel_package::PackageError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Engine(#[from] tysel_engine::EngineError),
    #[error(transparent)]
    Component(#[from] tysel_engine_wasm::ComponentError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[cfg(unix)]
    #[error(transparent)]
    TaskService(#[from] ModuleTaskServiceError),
    #[error(transparent)]
    LlmCapability(#[from] tysel_cap_llm::LlmError),
    #[error("LLM configuration: {0}")]
    Llm(String),
    #[error("invalid listen address '{0}'")]
    Listen(String),
    #[error("invalid component package: {0}")]
    ComponentPackage(&'static str),
}

pub async fn run_stub() -> Result<(), StubError> {
    run_tap(Tap::from_current_exe()?).await
}

pub async fn run_tap(tap: Tap) -> Result<(), StubError> {
    if !tap.components.is_empty() {
        if !tap.bundle.is_empty() {
            return Err(StubError::ComponentPackage(
                "mixed JavaScript and Component entrypoints are not supported",
            ));
        }
        let mut input = Vec::new();
        io::stdin().lock().take((MAX_COMPONENT_INPUT_BYTES + 1) as u64).read_to_end(&mut input)?;
        let input = std::str::from_utf8(&input).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "component input must be UTF-8 JSON")
        })?;
        let output = invoke_component_tap(&tap, input)?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(output.as_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        return Ok(());
    }
    let addr: SocketAddr =
        tap.manifest.listen.parse().map_err(|_| StubError::Listen(tap.manifest.listen.clone()))?;
    let config = IsolateConfig {
        memory_limit_bytes: tap.manifest.memory_limit_bytes,
        cpu_ms_per_turn: tap.manifest.cpu_ms_per_turn,
        request_timeout_ms: tap.manifest.request_timeout_ms,
    };
    let bundle = tap.bundle_source()?.to_owned();
    tysel_engine_qjs::configure_sqlite_path(&tap.manifest.sqlite_path, None);
    tysel_engine_qjs::configure_fs(
        tap.manifest.fs_read.clone(),
        tap.manifest.fs_write.clone(),
        None,
    );
    let postgres =
        tysel_manifest::resolve_postgres(&tap.manifest.postgres, &std::collections::HashMap::new());
    tysel_engine_qjs::configure_postgres(
        postgres.as_ref().map(|config| config.url.clone()),
        postgres.is_some_and(|config| config.read_only),
    );
    tysel_engine_qjs::configure_secrets(tysel_engine_qjs::load_declared(
        &tap.manifest.secret_names,
        &std::collections::HashMap::new(),
    ));
    configure_llm_from_env(tap.manifest.request_timeout_ms)?;
    tysel_engine_qjs::configure_fetch_hosts(tap.manifest.fetch_hosts.clone());
    tysel_engine_qjs::configure_execution_profile(&tap.manifest.execution_profile);
    tysel_observability::configure_http_log(&tap.manifest.application_id, tap.manifest.json_logs);
    let pool = spawn_app_isolate(
        &tap.manifest.execution_profile,
        &bundle,
        config,
        tap.manifest.secret_names.clone(),
    )?;
    let websocket = tap.manifest.websocket && !matches!(pool, AppIsolate::Isolated(_));
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    #[cfg(unix)]
    let task_service = {
        let socket_path =
            std::env::temp_dir().join(format!("tysel-task-{}.sock", std::process::id()));
        let service = ModuleTaskService::start(
            &socket_path,
            tap.manifest.application_id.clone(),
            bundle.clone(),
            config,
            tap.manifest.execution_profile.clone(),
            tap.manifest.secret_names.clone(),
        )
        .await?;
        if service.ingress().registry().is_empty() {
            service.shutdown().await?;
            None
        } else {
            println!("tysel task-rpc {}", socket_path.display());
            Some(service)
        }
    };
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    #[cfg(unix)]
    if let Some(service) = task_service {
        tokio::select! {
            result = serve_with_websocket(listener, pool, tap.manifest.max_request_bytes, websocket) => {
                service.shutdown().await?;
                result?;
            }
            error = service.failed() => {
                service.shutdown().await?;
                return Err(error.into());
            }
        }
    } else {
        serve_with_websocket(listener, pool, tap.manifest.max_request_bytes, websocket).await?;
    }
    #[cfg(not(unix))]
    serve_with_websocket(listener, pool, tap.manifest.max_request_bytes, websocket).await?;
    Ok(())
}

/// Invoke the single packaged Component through the portable source fallback.
/// AOT metadata is admitted now, but native deserialization remains disabled
/// until M5 package signatures can make Wasmtime's unsafe trust requirement.
pub fn invoke_component_tap(tap: &Tap, input: &str) -> Result<String, StubError> {
    let [component] = tap.components.as_slice() else {
        return Err(StubError::ComponentPackage("exactly one Component entrypoint is required"));
    };
    let engine = WasmComponentEngine::new(ComponentEngineConfig {
        max_memory_bytes: tap.manifest.memory_limit_bytes.clamp(1, MAX_COMPONENT_MEMORY_BYTES),
        max_execution_ms: tap.manifest.request_timeout_ms.clamp(1, MAX_COMPONENT_EXECUTION_MS),
        ..ComponentEngineConfig::default()
    })?;
    if let Some(aot) = component.aot.first() {
        let artifact = AotComponentRef {
            format_version: 1,
            component_abi_version: &component.abi_version,
            wasmtime_version: &aot.wasmtime_version,
            target: &aot.target,
            engine_compatibility_hash: aot.engine_compatibility_hash,
            source_sha256: aot.source_sha256,
            bytes: &aot.bytes,
        };
        let _aot_is_compatible = engine.validate_aot_ref(artifact, &component.source).is_ok();
    }
    let compiled = engine.compile(&component.source)?;
    compiled.authorize_imports(
        &tysel_capability::CapabilityRegistry::default(),
        tysel_capability::TrustMode::IsolatedTask,
        &BTreeSet::new(),
    )?;
    engine.invoke_json(&compiled, input).map_err(Into::into)
}

struct EngineSecretResolver;

impl SecretResolver for EngineSecretResolver {
    fn resolve(&self, handle: &str) -> Option<SecretValue> {
        tysel_engine_qjs::resolve_secret(handle).ok().and_then(|value| SecretValue::new(value).ok())
    }
}

struct RuntimeLlmAudit;

impl LlmAuditSink for RuntimeLlmAudit {
    fn record(&self, event: LlmAuditEvent) {
        tracing::info!(
            request_id = %event.request_id,
            model = %event.model,
            provider = %event.provider,
            input_bytes = event.input_bytes,
            output_bytes = event.output_bytes,
            elapsed_ms = event.elapsed_ms,
            outcome = ?event.outcome,
            "LLM capability"
        );
    }
}

pub fn configure_llm_from_env(request_timeout_ms: u64) -> Result<(), StubError> {
    let Ok(endpoint) = std::env::var("TYSEL_LLM_ENDPOINT") else {
        tysel_engine_qjs::configure_llm(None);
        return Ok(());
    };
    if endpoint.is_empty() {
        tysel_engine_qjs::configure_llm(None);
        return Ok(());
    }
    let upstream_model = std::env::var("TYSEL_LLM_MODEL").map_err(|_| {
        StubError::Llm("TYSEL_LLM_MODEL is required with TYSEL_LLM_ENDPOINT".into())
    })?;
    let alias = std::env::var("TYSEL_LLM_ALIAS").unwrap_or_else(|_| "default".into());
    let secret_name = std::env::var("TYSEL_LLM_SECRET").unwrap_or_else(|_| "OPENAI_API_KEY".into());
    let provider = Arc::new(OpenAiCompatibleProvider::new(&endpoint, Some(upstream_model))?);
    let gateway = LlmGateway::new(
        std::collections::BTreeMap::from([(
            alias,
            LlmRoute {
                provider_name: "openai-compatible".into(),
                provider,
                credential_handle: Some(format!("secret:{secret_name}")),
            },
        )]),
        Arc::new(EngineSecretResolver),
        Arc::new(RuntimeLlmAudit),
        LlmGatewayConfig {
            timeout_ms: request_timeout_ms.clamp(1, tysel_cap_llm::MAX_LLM_TIMEOUT_MS),
            ..LlmGatewayConfig::default()
        },
    )?;
    tysel_engine_qjs::configure_llm(Some(Arc::new(gateway)));
    Ok(())
}
