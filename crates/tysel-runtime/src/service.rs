use std::any::Any;
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tysel_cap_llm::{
    LlmAuditEvent, LlmAuditSink, LlmGateway, LlmGatewayConfig, LlmRoute, OpenAiCompatibleProvider,
    SecretResolver, SecretValue,
};
use tysel_capability::{CapabilityDescriptor, CapabilityId, TrustMode, effective_grants};
use tysel_engine::IsolateConfig;
use tysel_engine_wasm::{
    AotComponentRef, COMPONENT_ABI_VERSION, ComponentEngineConfig, MAX_COMPONENT_EXECUTION_MS,
    MAX_COMPONENT_INPUT_BYTES, MAX_COMPONENT_MEMORY_BYTES, StringCapabilityProvider,
    WasmComponentEngine,
};
use tysel_package::Tap;

use crate::http::{AppIsolate, HttpError, serve_with_limits, spawn_app_isolate};
use crate::{DurablePlane, DurablePlaneError};
#[cfg(unix)]
use crate::{ModuleTaskService, ModuleTaskServiceError};

static EMBEDDED_RUNTIME_INVENTORY: &[u8] =
    include_bytes!("../../tysel-build/src/runtime-components.json");

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
    #[error(transparent)]
    Observability(#[from] tysel_observability::OtlpInitError),
    #[error("LLM configuration: {0}")]
    Llm(String),
    #[error("invalid listen address '{0}'")]
    Listen(String),
    #[error("invalid component package: {0}")]
    ComponentPackage(&'static str),
    #[error("invalid Component deployment policy: {0}")]
    ComponentPolicy(String),
    #[error("invalid Component filesystem policy: {0}")]
    ComponentFilesystemPolicy(String),
    #[error(transparent)]
    Durable(#[from] DurablePlaneError),
}

/// Deployment-owned authority for a packaged Component. An empty policy is
/// intentionally fail-closed even when the package manifest requests access.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentRuntimePolicy {
    deployment_grants: BTreeSet<CapabilityId>,
    deployment_interfaces: BTreeSet<String>,
    filesystem_root: Option<PathBuf>,
}

impl ComponentRuntimePolicy {
    pub fn new(deployment_grants: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self {
            deployment_grants: deployment_grants.into_iter().collect(),
            deployment_interfaces: BTreeSet::new(),
            filesystem_root: None,
        }
    }

    /// Add operation-level deployment grants such as `tysel:fs/read` without
    /// approving every interface under the broader `tysel:fs` capability.
    pub fn with_interface_grants<I, S>(mut self, grants: I) -> Result<Self, StubError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for grant in grants {
            self.insert_interface_grant(grant.as_ref())?;
        }
        Ok(self)
    }

    pub fn with_filesystem_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.filesystem_root = Some(root.into());
        self
    }

    /// Read the deployment-owned capability allowlist. Only implemented
    /// built-ins are accepted so a typo cannot silently widen or weaken policy.
    pub fn from_environment() -> Result<Self, StubError> {
        let Some(raw) = std::env::var_os("TYSEL_COMPONENT_CAPABILITIES") else {
            return Ok(Self::default());
        };
        let raw = raw.into_string().map_err(|_| {
            StubError::ComponentPolicy("TYSEL_COMPONENT_CAPABILITIES must be UTF-8".into())
        })?;
        let (grants, interfaces) = parse_component_capability_grants(&raw)?;
        Ok(Self {
            deployment_grants: grants,
            deployment_interfaces: interfaces,
            filesystem_root: None,
        })
    }

    fn insert_interface_grant(&mut self, grant: &str) -> Result<(), StubError> {
        match grant {
            "tysel:fs/read" | "tysel:fs/write" => {
                self.deployment_interfaces.insert(grant.into());
                Ok(())
            }
            _ => Err(StubError::ComponentPolicy(format!(
                "unsupported capability interface '{grant}'"
            ))),
        }
    }

    fn allows_interface(&self, capability: &str, interface: &str) -> bool {
        self.deployment_grants.contains(&CapabilityId(capability.into()))
            || self.deployment_interfaces.contains(&format!("{capability}/{interface}"))
    }

    fn effective_capability_grants(&self) -> BTreeSet<CapabilityId> {
        let mut grants = self.deployment_grants.clone();
        if self.deployment_interfaces.iter().any(|grant| grant.starts_with("tysel:fs/")) {
            grants.insert(CapabilityId("tysel:fs".into()));
        }
        grants
    }
}

fn parse_component_capability_grants(
    raw: &str,
) -> Result<(BTreeSet<CapabilityId>, BTreeSet<String>), StubError> {
    let mut grants = BTreeSet::new();
    let mut interfaces = BTreeSet::new();
    for value in raw.split(',').map(str::trim).filter(|value| !value.is_empty()) {
        match value {
            "tysel:fs" => {
                grants.insert(CapabilityId(value.into()));
            }
            "tysel:fs/read" | "tysel:fs/write" => {
                interfaces.insert(value.into());
            }
            _ => {
                return Err(StubError::ComponentPolicy(format!(
                    "unsupported capability '{value}'"
                )));
            }
        }
    }
    Ok((grants, interfaces))
}

pub async fn run_stub() -> Result<(), StubError> {
    // Release evidence locates these exact bytes in the executable prefix, so
    // stale stubs cannot be paired with the current locked-graph SBOM.
    std::hint::black_box(EMBEDDED_RUNTIME_INVENTORY);
    run_tap(Tap::from_current_exe()?).await
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

pub async fn run_tap(tap: Tap) -> Result<(), StubError> {
    tysel_observability::configure_http_log(&tap.manifest.application_id, tap.manifest.json_logs);
    let _otlp = tysel_observability::configure_otlp(&tap.manifest.application_id)?;
    if !tap.components.is_empty() {
        if !tap.bundle.is_empty() {
            return Err(StubError::ComponentPackage(
                "mixed JavaScript and Component entrypoints are not supported",
            ));
        }
        let policy = ComponentRuntimePolicy::from_environment()?;
        return run_component_tap_with_policy(&tap, &policy);
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
    let pool = spawn_app_isolate(
        &tap.manifest.execution_profile,
        tap.manifest.workers,
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
    let durable = start_durable_plane(
        &tap.manifest.execution_profile,
        &tap.manifest.sqlite_path,
        None,
        &bundle,
        config,
    )?;
    if durable.is_some() {
        println!("tysel durable on");
    }
    println!("tysel listen {bound}");
    io::stdout().flush()?;
    #[cfg(unix)]
    if let Some(service) = task_service {
        tokio::select! {
            result = serve_with_limits(listener, pool, tap.manifest.max_request_bytes, tap.manifest.max_in_flight, websocket, tap.manifest.http1, tap.manifest.http2) => {
                let durable_shutdown = shutdown_durable(durable.as_ref()).await;
                service.shutdown().await?;
                durable_shutdown?;
                result?;
            }
            error = service.failed() => {
                let durable_shutdown = shutdown_durable(durable.as_ref()).await;
                service.shutdown().await?;
                durable_shutdown?;
                return Err(error.into());
            }
            signal = shutdown_signal() => {
                let durable_shutdown = shutdown_durable(durable.as_ref()).await;
                let shutdown = service.shutdown().await;
                signal?;
                durable_shutdown?;
                shutdown?;
            }
        }
    } else {
        tokio::select! {
            result = serve_with_limits(listener, pool, tap.manifest.max_request_bytes, tap.manifest.max_in_flight, websocket, tap.manifest.http1, tap.manifest.http2) => {
                shutdown_durable(durable.as_ref()).await?;
                result?;
            }
            signal = shutdown_signal() => {
                shutdown_durable(durable.as_ref()).await?;
                signal?;
            }
        }
    }
    #[cfg(not(unix))]
    tokio::select! {
        result = serve_with_limits(listener, pool, tap.manifest.max_request_bytes, tap.manifest.max_in_flight, websocket, tap.manifest.http1, tap.manifest.http2) => {
            shutdown_durable(durable.as_ref()).await?;
            result?;
        }
        signal = shutdown_signal() => {
            shutdown_durable(durable.as_ref()).await?;
            signal?;
        }
    }
    Ok(())
}

pub fn run_component_tap_with_policy(
    tap: &Tap,
    policy: &ComponentRuntimePolicy,
) -> Result<(), StubError> {
    let mut input = Vec::new();
    io::stdin().lock().take((MAX_COMPONENT_INPUT_BYTES + 1) as u64).read_to_end(&mut input)?;
    let input = std::str::from_utf8(&input).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "component input must be UTF-8 JSON")
    })?;
    let output = invoke_component_tap_with_policy(tap, input, policy)?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn start_durable_plane(
    execution_profile: &str,
    sqlite_path: &str,
    root: Option<&std::path::Path>,
    bundle: &str,
    config: IsolateConfig,
) -> Result<Option<std::sync::Arc<DurablePlane>>, StubError> {
    if execution_profile.eq_ignore_ascii_case("isolated") {
        return Ok(None);
    }
    if !DurablePlane::requested(sqlite_path, root, bundle, config)? {
        return Ok(None);
    }
    let Some(store) = DurablePlane::open_store(sqlite_path, root)? else {
        return Ok(None);
    };
    if !DurablePlane::should_start(store.as_ref(), bundle, config)? {
        return Ok(None);
    }
    let owner = format!("tysel-service-{}", std::process::id());
    Ok(Some(DurablePlane::start(store, bundle.to_owned(), config, owner)?))
}

async fn shutdown_durable(plane: Option<&std::sync::Arc<DurablePlane>>) -> Result<(), StubError> {
    if let Some(plane) = plane {
        plane.shutdown().await?;
    }
    Ok(())
}

/// Invoke the single packaged Component through the portable source fallback.
/// AOT metadata is admitted now, but native deserialization remains disabled
/// until the launcher passes an authenticated package-verification decision
/// into this runtime boundary.
pub fn invoke_component_tap(tap: &Tap, input: &str) -> Result<String, StubError> {
    invoke_component_tap_with_policy(tap, input, &ComponentRuntimePolicy::default())
}

/// Invoke a packaged Component with authority narrowed by all three software
/// layers: its actual imports, manifest requests, and deployment policy.
pub fn invoke_component_tap_with_policy(
    tap: &Tap,
    input: &str,
    policy: &ComponentRuntimePolicy,
) -> Result<String, StubError> {
    let [component] = tap.components.as_slice() else {
        return Err(StubError::ComponentPackage("exactly one Component entrypoint is required"));
    };
    if component.abi_version != COMPONENT_ABI_VERSION {
        return Err(StubError::ComponentPackage("unsupported Component ABI version"));
    }
    let engine = WasmComponentEngine::new(ComponentEngineConfig {
        max_memory_bytes: tap.manifest.memory_limit_bytes.clamp(1, MAX_COMPONENT_MEMORY_BYTES),
        max_execution_ms: tap.manifest.request_timeout_ms.clamp(1, MAX_COMPONENT_EXECUTION_MS),
        ..ComponentEngineConfig::default()
    })?;
    let compatible_aot = component.aot.iter().any(|aot| {
        let artifact = AotComponentRef {
            format_version: 1,
            component_abi_version: &component.abi_version,
            wasmtime_version: &aot.wasmtime_version,
            target: &aot.target,
            engine_compatibility_hash: aot.engine_compatibility_hash,
            source_sha256: aot.source_sha256,
            bytes: &aot.bytes,
        };
        engine.validate_aot_ref(artifact, &component.source).is_ok()
    });
    tracing::debug!(
        compatible_aot,
        "using portable Component source; native AOT loading requires authenticated trust handoff"
    );
    let compiled = engine.compile(&component.source)?;
    let build_grants = compiled.required_imports().iter().map(|import| import.id.clone()).collect();
    let application_grants = component_application_grants(tap);
    let deployment_grants = policy.effective_capability_grants();
    let grants = effective_grants(&build_grants, &application_grants, &deployment_grants);
    let providers = component_capability_providers(tap, policy)?;
    engine
        .invoke_json_with_capabilities(
            &compiled,
            input,
            &providers,
            TrustMode::IsolatedTask,
            &grants,
        )
        .map_err(Into::into)
}

fn component_application_grants(tap: &Tap) -> BTreeSet<CapabilityId> {
    let mut grants = BTreeSet::new();
    if !tap.manifest.fs_read.is_empty() || !tap.manifest.fs_write.is_empty() {
        grants.insert(CapabilityId("tysel:fs".into()));
    }
    grants
}

fn component_capability_providers(
    tap: &Tap,
    policy: &ComponentRuntimePolicy,
) -> Result<Vec<StringCapabilityProvider>, StubError> {
    let allow_read =
        !tap.manifest.fs_read.is_empty() && policy.allows_interface("tysel:fs", "read");
    let allow_write =
        !tap.manifest.fs_write.is_empty() && policy.allows_interface("tysel:fs", "write");
    if !allow_read && !allow_write {
        return Ok(Vec::new());
    }
    let timeout =
        Duration::from_millis(tap.manifest.request_timeout_ms.clamp(1, MAX_COMPONENT_EXECUTION_MS));
    let deadline = Instant::now().checked_add(timeout).unwrap_or_else(Instant::now);
    let audit = tysel_observability::CapabilityLogger::new(
        tap.manifest.application_id.clone(),
        tap.manifest.json_logs,
    );
    let request_id = tysel_observability::next_request_id();
    let read_roots = if allow_read { tap.manifest.fs_read.clone() } else { Vec::new() };
    let write_roots = if allow_write { tap.manifest.fs_write.clone() } else { Vec::new() };
    let filesystem_root = policy.filesystem_root.clone();
    let remaining = remaining_host_io(deadline).map_err(StubError::ComponentFilesystemPolicy)?;
    let filesystem = Arc::new(
        run_bounded_host_io(remaining, move || {
            tysel_cap_fs::Filesystem::new(read_roots, write_roots, filesystem_root.as_deref())
        })
        .map_err(StubError::ComponentFilesystemPolicy)?,
    );
    let mut providers = Vec::new();
    if allow_read {
        let filesystem = Arc::clone(&filesystem);
        let audit = audit.clone();
        providers.push(
            StringCapabilityProvider::new(
                CapabilityDescriptor::new(
                    "tysel:fs/read@0.4.0".parse().expect("static capability import"),
                    [TrustMode::IsolatedTask],
                )
                .expect("static capability descriptor"),
                "call",
                move |input| {
                    let path: String = serde_json::from_str(input)
                        .map_err(|_| "filesystem read input must be a JSON string".to_owned())?;
                    let filesystem = Arc::clone(&filesystem);
                    run_bounded_host_io(remaining_host_io(deadline)?, move || {
                        let contents = filesystem.read(&path)?;
                        serde_json::to_string(&contents).map_err(|error| error.to_string())
                    })
                },
            )?
            .with_audit(move |result, elapsed| {
                audit.log("fs", "read", result, elapsed, request_id);
            }),
        );
    }
    if allow_write {
        let filesystem = Arc::clone(&filesystem);
        let audit = audit.clone();
        providers.push(
            StringCapabilityProvider::new(
                CapabilityDescriptor::new(
                    "tysel:fs/write@0.4.0".parse().expect("static capability import"),
                    [TrustMode::IsolatedTask],
                )
                .expect("static capability descriptor"),
                "call",
                move |input| {
                    let value: serde_json::Value = serde_json::from_str(input)
                        .map_err(|_| "filesystem write input must be JSON".to_owned())?;
                    let object = value
                        .as_object()
                        .ok_or_else(|| "filesystem write input must be an object".to_owned())?;
                    if object.len() != 2 {
                        return Err("filesystem write input must contain only path and data".into());
                    }
                    let path = object
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| "filesystem write path must be a string".to_owned())?
                        .to_owned();
                    let data = object
                        .get("data")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| "filesystem write data must be a string".to_owned())?
                        .to_owned();
                    let filesystem = Arc::clone(&filesystem);
                    run_bounded_host_io(remaining_host_io(deadline)?, move || {
                        filesystem.write(&path, &data)?;
                        Ok("null".into())
                    })
                },
            )?
            .with_audit(move |result, elapsed| {
                audit.log("fs", "write", result, elapsed, request_id);
            }),
        );
    }
    Ok(providers)
}

const HOST_IO_WORKERS: usize = 4;
const HOST_IO_QUEUE_CAPACITY: usize = 32;

type HostIoValue = Box<dyn Any + Send + 'static>;
type HostIoTask = Box<dyn FnOnce() -> Result<HostIoValue, String> + Send + 'static>;

struct HostIoJob {
    task: HostIoTask,
    response: SyncSender<Result<HostIoValue, String>>,
    cancelled: Arc<AtomicBool>,
}

static HOST_IO: OnceLock<Result<SyncSender<HostIoJob>, String>> = OnceLock::new();

fn run_bounded_host_io<T>(
    timeout: Duration,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Any + Send + 'static,
{
    let sender = host_io_sender()?;
    let (response, receiver) = sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let task = Box::new(move || task().map(|value| Box::new(value) as HostIoValue));
    let job = HostIoJob { task, response, cancelled: Arc::clone(&cancelled) };
    sender.try_send(job).map_err(|error| match error {
        TrySendError::Full(_) => "filesystem executor is saturated".to_owned(),
        TrySendError::Disconnected(_) => "filesystem executor is unavailable".to_owned(),
    })?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.and_then(|value| {
            value
                .downcast::<T>()
                .map(|value| *value)
                .map_err(|_| "filesystem executor returned an invalid result".into())
        }),
        Err(RecvTimeoutError::Timeout) => {
            cancelled.store(true, Ordering::Release);
            Err("filesystem operation timed out".into())
        }
        Err(RecvTimeoutError::Disconnected) => Err("filesystem executor is unavailable".into()),
    }
}

fn remaining_host_io(deadline: Instant) -> Result<Duration, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { Err("filesystem operation timed out".into()) } else { Ok(remaining) }
}

fn host_io_sender() -> Result<SyncSender<HostIoJob>, String> {
    HOST_IO.get_or_init(start_host_io_workers).clone()
}

fn start_host_io_workers() -> Result<SyncSender<HostIoJob>, String> {
    let (sender, receiver) = sync_channel::<HostIoJob>(HOST_IO_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let mut started = 0;
    for index in 0..HOST_IO_WORKERS {
        let receiver = Arc::clone(&receiver);
        if std::thread::Builder::new()
            .name(format!("tysel-component-io-{index}"))
            .spawn(move || host_io_worker(&receiver))
            .is_ok()
        {
            started += 1;
        }
    }
    if started == 0 { Err("failed to start filesystem executor".into()) } else { Ok(sender) }
}

fn host_io_worker(receiver: &Mutex<Receiver<HostIoJob>>) {
    loop {
        let job = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else {
            return;
        };
        if job.cancelled.load(Ordering::Acquire) {
            continue;
        }
        let result = (job.task)();
        if !job.cancelled.load(Ordering::Acquire) {
            let _ = job.response.send(result);
        }
    }
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

#[cfg(test)]
mod component_host_io_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[test]
    fn component_capability_parser_separates_broad_and_interface_grants() {
        let (grants, interfaces) =
            super::parse_component_capability_grants(" tysel:fs/read,tysel:fs,tysel:fs/write ")
                .unwrap();

        assert_eq!(grants, [super::CapabilityId("tysel:fs".into())].into());
        assert_eq!(interfaces, ["tysel:fs/read".to_owned(), "tysel:fs/write".to_owned()].into());
    }

    #[test]
    fn component_capability_parser_rejects_unknown_grants() {
        let error = super::parse_component_capability_grants("tysel:fs/delete").unwrap_err();
        assert!(error.to_string().contains("unsupported capability 'tysel:fs/delete'"));
    }

    #[test]
    fn broad_component_grant_remains_broad_when_combined_with_an_interface_grant() {
        let exact = super::ComponentRuntimePolicy::default()
            .with_interface_grants(["tysel:fs/read"])
            .unwrap();
        assert!(exact.allows_interface("tysel:fs", "read"));
        assert!(!exact.allows_interface("tysel:fs", "write"));

        let mixed = super::ComponentRuntimePolicy::new([super::CapabilityId("tysel:fs".into())])
            .with_interface_grants(["tysel:fs/read"])
            .unwrap();
        assert!(mixed.allows_interface("tysel:fs", "read"));
        assert!(mixed.allows_interface("tysel:fs", "write"));
    }

    #[test]
    fn blocking_host_io_returns_at_its_deadline() {
        let completed = Arc::new(AtomicBool::new(false));
        let completed_in_task = Arc::clone(&completed);
        let error = super::run_bounded_host_io(Duration::from_millis(10), move || {
            std::thread::sleep(Duration::from_millis(100));
            completed_in_task.store(true, Ordering::Release);
            Ok::<String, String>("late".into())
        })
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(!completed.load(Ordering::Acquire));
    }
}
