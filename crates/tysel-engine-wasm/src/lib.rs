//! Bounded Wasmtime Component execution engine (M4).
//!
//! Components receive no application capabilities by default. A small WASI
//! runtime profile may be linked for language support, with closed stdio, no
//! arguments or environment, no preopened directories, and denied networking.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tysel_capability::{
    CapabilityDescriptor, CapabilityId, CapabilityImport, CapabilityRegistry,
    CapabilityRegistryError, TrustMode,
};
use wasmtime::component::types::{ComponentItem, Type};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Precompiled, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{IoView, WasiCtx, WasiCtxBuilder, WasiView};

#[allow(dead_code)]
mod wit_contract {
    wasmtime::component::bindgen!({
        path: "../../wit/component",
        world: "task",
    });
}

pub const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_COMPONENT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_COMPONENT_FUEL: u64 = 100_000_000;
pub const MAX_COMPONENT_EXECUTION_MS: u64 = 60_000;
pub const MAX_COMPONENT_ERROR_BYTES: usize = 4 * 1024;
pub const MAX_COMPONENT_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_COMPONENT_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_AOT_COMPONENT_BYTES: usize = 64 * 1024 * 1024;
pub const COMPONENT_ABI_VERSION: &str = "0.4.0";
pub const WASMTIME_VERSION: &str = "32.0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentEngineConfig {
    pub max_component_bytes: usize,
    pub max_memory_bytes: usize,
    pub fuel: u64,
    pub max_execution_ms: u64,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for ComponentEngineConfig {
    fn default() -> Self {
        Self {
            max_component_bytes: MAX_COMPONENT_BYTES,
            max_memory_bytes: MAX_COMPONENT_MEMORY_BYTES,
            fuel: 10_000_000,
            max_execution_ms: 30_000,
            max_input_bytes: MAX_COMPONENT_INPUT_BYTES,
            max_output_bytes: MAX_COMPONENT_OUTPUT_BYTES,
        }
    }
}

#[derive(Clone)]
pub struct WasmComponentEngine {
    engine: Engine,
    config: ComponentEngineConfig,
    _epoch_ticker: Arc<EpochTicker>,
}

struct EpochTicker;

#[derive(Clone)]
pub struct CompiledComponent {
    component: Component,
    source_sha256: [u8; 32],
    required_imports: Vec<CapabilityImport>,
    wasi_runtime_imports: Vec<String>,
}

/// Host-specific Wasmtime output produced at package build time. The portable
/// source component remains the authority for identity and safe fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AotComponent {
    pub format_version: u32,
    pub component_abi_version: String,
    pub wasmtime_version: String,
    pub target: String,
    pub engine_compatibility_hash: u64,
    pub source_sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct AotComponentRef<'a> {
    pub format_version: u32,
    pub component_abi_version: &'a str,
    pub wasmtime_version: &'a str,
    pub target: &'a str,
    pub engine_compatibility_hash: u64,
    pub source_sha256: [u8; 32],
    pub bytes: &'a [u8],
}

impl AotComponent {
    pub fn as_ref(&self) -> AotComponentRef<'_> {
        AotComponentRef {
            format_version: self.format_version,
            component_abi_version: &self.component_abi_version,
            wasmtime_version: &self.wasmtime_version,
            target: &self.target,
            engine_compatibility_hash: self.engine_compatibility_hash,
            source_sha256: self.source_sha256,
            bytes: &self.bytes,
        }
    }
}

type StringCapabilityHandler = dyn Fn(&str) -> Result<String, String> + Send + Sync + 'static;
type StringCapabilityAudit = dyn Fn(&str, Duration) + Send + Sync + 'static;

/// One versioned interface implementation for string/result-string WIT
/// capabilities. The registry selects the provider before its function is
/// inserted into the Wasmtime linker.
#[derive(Clone)]
pub struct StringCapabilityProvider {
    descriptor: CapabilityDescriptor,
    function: String,
    handler: Arc<StringCapabilityHandler>,
    audit: Option<Arc<StringCapabilityAudit>>,
}

impl StringCapabilityProvider {
    pub fn new(
        descriptor: CapabilityDescriptor,
        function: impl Into<String>,
        handler: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static,
    ) -> Result<Self, ComponentError> {
        let function = function.into();
        if function.is_empty()
            || function.len() > 128
            || !function
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ComponentError::InvalidCapabilityFunction(function));
        }
        Ok(Self { descriptor, function, handler: Arc::new(handler), audit: None })
    }

    /// Observe the final capability result after the engine's input and output
    /// limits have been applied.
    pub fn with_audit(mut self, audit: impl Fn(&str, Duration) + Send + Sync + 'static) -> Self {
        self.audit = Some(Arc::new(audit));
        self
    }

    pub fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }
}

impl CompiledComponent {
    pub fn source_sha256(&self) -> [u8; 32] {
        self.source_sha256
    }

    /// Version-qualified capability interfaces requested by the component,
    /// sorted by import name for stable policy and audit decisions.
    pub fn required_imports(&self) -> &[CapabilityImport] {
        &self.required_imports
    }

    /// Restricted WASI interfaces needed by a language runtime. These are not
    /// application capabilities and never inherit host stdio, environment,
    /// arguments, directories, or network authority.
    pub fn wasi_runtime_imports(&self) -> &[String] {
        &self.wasi_runtime_imports
    }

    /// Resolve every requested interface against the effective grant set.
    /// This is deliberately separate from linking an implementation: policy
    /// admission must complete before host functions are made reachable.
    pub fn authorize_imports(
        &self,
        registry: &CapabilityRegistry,
        trust_mode: TrustMode,
        effective_grants: &BTreeSet<CapabilityId>,
    ) -> Result<Vec<CapabilityImport>, ComponentError> {
        self.required_imports
            .iter()
            .map(|requested| {
                registry
                    .resolve(requested, trust_mode, effective_grants)
                    .map(|descriptor| descriptor.import.clone())
                    .map_err(ComponentError::capability)
            })
            .collect()
    }
}

struct StoreState {
    limits: StoreLimits,
    table: ResourceTable,
    wasi: WasiCtx,
}

impl IoView for StoreState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiView for StoreState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl WasmComponentEngine {
    pub fn new(config: ComponentEngineConfig) -> Result<Self, ComponentError> {
        validate_config(config)?;
        let mut runtime = Config::new();
        runtime.wasm_component_model(true).consume_fuel(true).epoch_interruption(true);
        let engine = Engine::new(&runtime).map_err(ComponentError::runtime)?;
        let epoch_ticker = Arc::new(EpochTicker);
        let alive = Arc::downgrade(&epoch_ticker);
        let ticker_engine = engine.clone();
        std::thread::Builder::new()
            .name("tysel-wasm-epoch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(10));
                    if alive.upgrade().is_none() {
                        break;
                    }
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|error| ComponentError::Runtime(bounded_display(error)))?;
        Ok(Self { engine, config, _epoch_ticker: epoch_ticker })
    }

    /// Compile a Component binary. Core WebAssembly modules are not accepted,
    /// and oversized payloads fail before Wasmtime parses or allocates them.
    pub fn compile(&self, bytes: &[u8]) -> Result<CompiledComponent, ComponentError> {
        if bytes.len() > self.config.max_component_bytes {
            return Err(ComponentError::ComponentTooLarge {
                actual: bytes.len(),
                maximum: self.config.max_component_bytes,
            });
        }
        let source_sha256 = Sha256::digest(bytes).into();
        let component = Component::new(&self.engine, bytes).map_err(ComponentError::compile)?;
        validate_task_export(&component, &self.engine)?;
        let mut required_imports = Vec::new();
        let mut wasi_runtime_imports = Vec::new();
        for (name, _) in component.component_type().imports(&self.engine) {
            if name.starts_with("wasi:") {
                validate_wasi_runtime_import(name)?;
                wasi_runtime_imports.push(name.into());
            } else {
                required_imports.push(
                    name.parse::<CapabilityImport>()
                        .map_err(|_| ComponentError::InvalidCapabilityImport(name.into()))?,
                );
            }
        }
        required_imports.sort();
        required_imports.dedup();
        wasi_runtime_imports.sort();
        wasi_runtime_imports.dedup();
        Ok(CompiledComponent { component, source_sha256, required_imports, wasi_runtime_imports })
    }

    /// Produce a host-specific AOT image. Loading arbitrary serialized native
    /// code is intentionally not exposed here; TAP admission must verify its
    /// package integrity before a later runtime layer may deserialize it.
    pub fn precompile(&self, bytes: &[u8]) -> Result<AotComponent, ComponentError> {
        let compiled = self.compile(bytes)?;
        let aot = self.engine.precompile_component(bytes).map_err(ComponentError::aot)?;
        if aot.len() > MAX_AOT_COMPONENT_BYTES {
            return Err(ComponentError::AotTooLarge {
                actual: aot.len(),
                maximum: MAX_AOT_COMPONENT_BYTES,
            });
        }
        if Engine::detect_precompiled(&aot) != Some(Precompiled::Component) {
            return Err(ComponentError::Aot("Wasmtime returned a non-component artifact".into()));
        }
        Ok(AotComponent {
            format_version: 1,
            component_abi_version: COMPONENT_ABI_VERSION.into(),
            wasmtime_version: WASMTIME_VERSION.into(),
            target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            engine_compatibility_hash: self.engine_compatibility_hash(),
            source_sha256: compiled.source_sha256,
            bytes: aot,
        })
    }

    /// Reject AOT images built for another engine configuration or source.
    /// This is an admission check, not a substitute for package signatures.
    pub fn validate_aot(
        &self,
        artifact: &AotComponent,
        source: &[u8],
    ) -> Result<(), ComponentError> {
        self.validate_aot_ref(artifact.as_ref(), source)
    }

    pub fn validate_aot_ref(
        &self,
        artifact: AotComponentRef<'_>,
        source: &[u8],
    ) -> Result<(), ComponentError> {
        let expected_source: [u8; 32] = Sha256::digest(source).into();
        let expected_target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
        if artifact.format_version != 1
            || artifact.component_abi_version != COMPONENT_ABI_VERSION
            || artifact.wasmtime_version != WASMTIME_VERSION
            || artifact.target != expected_target
            || artifact.engine_compatibility_hash != self.engine_compatibility_hash()
            || artifact.source_sha256 != expected_source
            || artifact.bytes.len() > MAX_AOT_COMPONENT_BYTES
            || Engine::detect_precompiled(artifact.bytes) != Some(Precompiled::Component)
        {
            return Err(ComponentError::IncompatibleAot);
        }
        Ok(())
    }

    fn engine_compatibility_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.engine.precompile_compatibility_hash().hash(&mut hasher);
        hasher.finish()
    }

    /// Invoke `tysel:component/task@0.4.0`'s
    /// `run(input: string) -> result<string, string>` export.
    ///
    /// No application capabilities are linked. Only the restricted WASI
    /// language-runtime profile requested by the component is available.
    pub fn invoke_json(
        &self,
        component: &CompiledComponent,
        input: &str,
    ) -> Result<String, ComponentError> {
        let mut linker = Linker::<StoreState>::new(&self.engine);
        self.link_restricted_wasi(component, &mut linker)?;
        self.invoke_with_linker(component, input, linker)
    }

    /// Admit and link only registry-selected interface providers. Providers
    /// are linked under the exact version requested by the guest while their
    /// implementation version follows registry compatibility rules.
    pub fn invoke_json_with_capabilities(
        &self,
        component: &CompiledComponent,
        input: &str,
        providers: &[StringCapabilityProvider],
        trust_mode: TrustMode,
        effective_grants: &BTreeSet<CapabilityId>,
    ) -> Result<String, ComponentError> {
        let registry =
            CapabilityRegistry::new(providers.iter().map(|provider| provider.descriptor.clone()))
                .map_err(ComponentError::capability)?;
        let resolved = component.authorize_imports(&registry, trust_mode, effective_grants)?;
        let mut linker = Linker::<StoreState>::new(&self.engine);
        self.link_restricted_wasi(component, &mut linker)?;
        for (requested, selected) in component.required_imports.iter().zip(resolved) {
            let provider = providers
                .iter()
                .find(|provider| provider.descriptor.import == selected)
                .ok_or_else(|| ComponentError::MissingCapabilityProvider(selected.to_string()))?;
            let handler = Arc::clone(&provider.handler);
            let audit = provider.audit.clone();
            let function = provider.function.clone();
            let max_input_bytes = self.config.max_input_bytes;
            let max_output_bytes = self.config.max_output_bytes;
            linker
                .instance(&requested.to_string())
                .map_err(ComponentError::link)?
                .func_wrap(&function, move |_store, (input,): (String,)| {
                    let result = invoke_string_capability(
                        handler.as_ref(),
                        audit.as_deref(),
                        &input,
                        max_input_bytes,
                        max_output_bytes,
                    );
                    Ok((result,))
                })
                .map_err(ComponentError::link)?;
        }
        self.invoke_with_linker(component, input, linker)
    }

    fn link_restricted_wasi(
        &self,
        component: &CompiledComponent,
        linker: &mut Linker<StoreState>,
    ) -> Result<(), ComponentError> {
        if !component.wasi_runtime_imports.is_empty() {
            wasmtime_wasi::add_to_linker_sync(linker)
                .map_err(|error| ComponentError::Link(bounded_display(error)))?;
        }
        Ok(())
    }

    fn invoke_with_linker(
        &self,
        component: &CompiledComponent,
        input: &str,
        linker: Linker<StoreState>,
    ) -> Result<String, ComponentError> {
        if input.len() > self.config.max_input_bytes {
            return Err(ComponentError::InputTooLarge {
                actual: input.len(),
                maximum: self.config.max_input_bytes,
            });
        }
        serde_json::from_str::<serde_json::Value>(input)
            .map_err(|error| ComponentError::InvalidInputJson(bounded_display(error)))?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .instances(32)
            .memories(8)
            .tables(8)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreState { limits, table: ResourceTable::new(), wasi: WasiCtxBuilder::new().build() },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.config.fuel).map_err(ComponentError::runtime)?;
        store.set_epoch_deadline(self.config.max_execution_ms.div_ceil(10));
        store.epoch_deadline_trap();

        let instance = linker
            .instantiate(&mut store, &component.component)
            .map_err(ComponentError::instantiate)?;
        let run = instance
            .get_typed_func::<(String,), (Result<String, String>,)>(&mut store, "run")
            .map_err(ComponentError::abi)?;
        let (result,) =
            run.call(&mut store, (input.to_owned(),)).map_err(ComponentError::invoke)?;
        run.post_return(&mut store).map_err(ComponentError::invoke)?;
        let output = match result {
            Ok(output) => output,
            Err(error) => return Err(ComponentError::Guest(bounded_message(error))),
        };
        if output.len() > self.config.max_output_bytes {
            return Err(ComponentError::OutputTooLarge {
                actual: output.len(),
                maximum: self.config.max_output_bytes,
            });
        }
        serde_json::from_str::<serde_json::Value>(&output)
            .map_err(|error| ComponentError::InvalidOutputJson(bounded_display(error)))?;
        Ok(output)
    }
}

fn invoke_string_capability(
    handler: &StringCapabilityHandler,
    audit: Option<&StringCapabilityAudit>,
    input: &str,
    max_input_bytes: usize,
    max_output_bytes: usize,
) -> Result<String, String> {
    let started = Instant::now();
    let result = if input.len() > max_input_bytes {
        Err(format!("capability input exceeds {max_input_bytes} bytes"))
    } else {
        handler(input)
    }
    .map_err(bounded_message)
    .and_then(|output| {
        if output.len() > max_output_bytes {
            Err(format!("capability output exceeds {max_output_bytes} bytes"))
        } else {
            Ok(output)
        }
    });
    if let Some(audit) = audit {
        audit(if result.is_ok() { "ok" } else { "error" }, started.elapsed());
    }
    result
}

const ALLOWED_WASI_RUNTIME_INTERFACES: &[&str] = &[
    "wasi:cli/environment",
    "wasi:cli/exit",
    "wasi:cli/stdin",
    "wasi:cli/stdout",
    "wasi:cli/stderr",
    "wasi:cli/terminal-input",
    "wasi:cli/terminal-output",
    "wasi:cli/terminal-stdin",
    "wasi:cli/terminal-stdout",
    "wasi:cli/terminal-stderr",
    "wasi:clocks/monotonic-clock",
    "wasi:clocks/wall-clock",
    "wasi:filesystem/types",
    "wasi:filesystem/preopens",
    "wasi:io/error",
    "wasi:io/poll",
    "wasi:io/streams",
    "wasi:random/random",
];

fn validate_wasi_runtime_import(name: &str) -> Result<(), ComponentError> {
    let Some((interface, version)) = name.rsplit_once('@') else {
        return Err(ComponentError::InvalidWasiRuntimeImport(name.into()));
    };
    let version = version
        .parse::<tysel_capability::AbiVersion>()
        .map_err(|_| ComponentError::InvalidWasiRuntimeImport(name.into()))?;
    if version.major != 0
        || version.minor != 2
        || !ALLOWED_WASI_RUNTIME_INTERFACES.contains(&interface)
    {
        return Err(ComponentError::InvalidWasiRuntimeImport(name.into()));
    }
    Ok(())
}

fn validate_task_export(component: &Component, engine: &Engine) -> Result<(), ComponentError> {
    let component_type = component.component_type();
    let Some(ComponentItem::ComponentFunc(run)) = component_type.get_export(engine, "run") else {
        return Err(ComponentError::Abi(
            "missing `run` export for tysel:component/task@0.4.0".into(),
        ));
    };
    let params = run.params().collect::<Vec<_>>();
    let results = run.results().collect::<Vec<_>>();
    let valid_result = match results.as_slice() {
        [Type::Result(result)] => {
            result.ok() == Some(Type::String) && result.err() == Some(Type::String)
        }
        _ => false,
    };
    if params.as_slice() != [("input", Type::String)] || !valid_result {
        return Err(ComponentError::Abi(
            "`run` must be (input: string) -> result<string, string>".into(),
        ));
    }
    Ok(())
}

fn validate_config(config: ComponentEngineConfig) -> Result<(), ComponentError> {
    if config.max_component_bytes == 0 || config.max_component_bytes > MAX_COMPONENT_BYTES {
        return Err(ComponentError::InvalidConfig("invalid component byte limit"));
    }
    if config.max_memory_bytes == 0 || config.max_memory_bytes > MAX_COMPONENT_MEMORY_BYTES {
        return Err(ComponentError::InvalidConfig("invalid component memory limit"));
    }
    if config.fuel == 0 || config.fuel > MAX_COMPONENT_FUEL {
        return Err(ComponentError::InvalidConfig("invalid component fuel limit"));
    }
    if config.max_execution_ms == 0 || config.max_execution_ms > MAX_COMPONENT_EXECUTION_MS {
        return Err(ComponentError::InvalidConfig("invalid component execution timeout"));
    }
    if config.max_input_bytes == 0 || config.max_input_bytes > MAX_COMPONENT_INPUT_BYTES {
        return Err(ComponentError::InvalidConfig("invalid component input limit"));
    }
    if config.max_output_bytes == 0 || config.max_output_bytes > MAX_COMPONENT_OUTPUT_BYTES {
        return Err(ComponentError::InvalidConfig("invalid component output limit"));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("invalid component engine configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("component is {actual} bytes; maximum is {maximum}")]
    ComponentTooLarge { actual: usize, maximum: usize },
    #[error("component input is {actual} bytes; maximum is {maximum}")]
    InputTooLarge { actual: usize, maximum: usize },
    #[error("component output is {actual} bytes; maximum is {maximum}")]
    OutputTooLarge { actual: usize, maximum: usize },
    #[error("precompiled component is {actual} bytes; maximum is {maximum}")]
    AotTooLarge { actual: usize, maximum: usize },
    #[error("component import is not a version-qualified capability: '{0}'")]
    InvalidCapabilityImport(String),
    #[error("component WASI runtime import is not allowed: '{0}'")]
    InvalidWasiRuntimeImport(String),
    #[error("component capability admission failed: {0}")]
    Capability(String),
    #[error("invalid capability function name '{0}'")]
    InvalidCapabilityFunction(String),
    #[error("selected capability provider is missing: '{0}'")]
    MissingCapabilityProvider(String),
    #[error("component capability linking failed: {0}")]
    Link(String),
    #[error("component input is not valid JSON: {0}")]
    InvalidInputJson(String),
    #[error("component output is not valid JSON: {0}")]
    InvalidOutputJson(String),
    #[error("component compilation failed: {0}")]
    Compile(String),
    #[error("component instantiation failed: {0}")]
    Instantiate(String),
    #[error("component ABI mismatch: {0}")]
    Abi(String),
    #[error("component invocation failed: {0}")]
    Invoke(String),
    #[error("component returned an error: {0}")]
    Guest(String),
    #[error("component runtime failed: {0}")]
    Runtime(String),
    #[error("component AOT compilation failed: {0}")]
    Aot(String),
    #[error("component AOT artifact is incompatible")]
    IncompatibleAot,
}

impl ComponentError {
    fn compile(error: wasmtime::Error) -> Self {
        Self::Compile(bounded_error(error))
    }

    fn instantiate(error: wasmtime::Error) -> Self {
        Self::Instantiate(bounded_error(error))
    }

    fn abi(error: wasmtime::Error) -> Self {
        Self::Abi(bounded_error(error))
    }

    fn invoke(error: wasmtime::Error) -> Self {
        Self::Invoke(bounded_error(error))
    }

    fn runtime(error: wasmtime::Error) -> Self {
        Self::Runtime(bounded_error(error))
    }

    fn aot(error: wasmtime::Error) -> Self {
        Self::Aot(bounded_error(error))
    }

    fn capability(error: CapabilityRegistryError) -> Self {
        Self::Capability(bounded_display(error))
    }

    fn link(error: wasmtime::Error) -> Self {
        Self::Link(bounded_error(error))
    }
}

fn bounded_error(error: wasmtime::Error) -> String {
    bounded_message(format!("{error:#}"))
}

fn bounded_display(error: impl std::fmt::Display) -> String {
    bounded_message(error.to_string())
}

fn bounded_message(mut message: String) -> String {
    if message.len() > MAX_COMPONENT_ERROR_BYTES {
        let mut end = MAX_COMPONENT_ERROR_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const ECHO_COMPONENT: &str = r#"
(component
  (core module $module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc")
      (param $old-ptr i32) (param $old-len i32) (param $align i32) (param $new-len i32)
      (result i32)
      (local $ptr i32)
      global.get $heap
      local.tee $ptr
      local.get $new-len
      i32.add
      global.set $heap
      local.get $ptr)
    (func (export "run") (param $ptr i32) (param $len i32) (result i32)
      i32.const 0
      i32.const 0
      i32.store
      i32.const 4
      local.get $ptr
      i32.store
      i32.const 8
      local.get $len
      i32.store
      i32.const 0))
  (core instance $instance (instantiate $module))
  (alias core export $instance "memory" (core memory $memory))
  (alias core export $instance "realloc" (core func $realloc))
  (alias core export $instance "run" (core func $run-core))
  (type $run-type
    (func (param "input" string) (result (result string (error string)))))
  (func $run (type $run-type)
    (canon lift (core func $run-core) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#;

    fn component(text: &str) -> Vec<u8> {
        wat::parse_str(text).unwrap()
    }

    fn echo_component_with_import() -> Vec<u8> {
        component(&ECHO_COMPONENT.replacen(
            "(component",
            r#"(component
  (type $host (func (param "input" string) (result (result string (error string)))))
  (type $host-instance (instance (export "call" (func (type $host)))))
  (import "tysel:test/host@0.4.0" (instance $host-import (type $host-instance)))"#,
            1,
        ))
    }

    fn proxy_component_with_import() -> Vec<u8> {
        component(
            r#"
(component
  (type $host (func (param "input" string) (result (result string (error string)))))
  (type $host-instance (instance (export "call" (func (type $host)))))
  (import "tysel:test/host@0.4.0" (instance $host-import (type $host-instance)))
  (alias export $host-import "call" (func $call))
  (core module $memory-module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc")
      (param $old-ptr i32) (param $old-len i32) (param $align i32) (param $new-len i32)
      (result i32)
      (local $ptr i32)
      global.get $heap
      local.tee $ptr
      local.get $new-len
      i32.add
      global.set $heap
      local.get $ptr))
  (core instance $memory-instance (instantiate $memory-module))
  (alias core export $memory-instance "memory" (core memory $memory))
  (alias core export $memory-instance "realloc" (core func $realloc))
  (core func $lowered-call
    (canon lower (func $call) (memory $memory) (realloc $realloc)))
  (core module $adapter
    (import "host" "call" (func $host-call (param i32 i32 i32)))
    (func (export "call") (param $ptr i32) (param $len i32) (result i32)
      local.get $ptr
      local.get $len
      i32.const 0
      call $host-call
      i32.const 0))
  (core instance $host-core
    (export "call" (func $lowered-call)))
  (core instance $adapter-instance
    (instantiate $adapter (with "host" (instance $host-core))))
  (alias core export $adapter-instance "call" (core func $adapter-call))
  (func $run (type $host)
    (canon lift (core func $adapter-call) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#,
        )
    }

    #[test]
    fn invokes_the_wit_json_string_abi() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let bytes = component(ECHO_COMPONENT);
        let compiled = engine.compile(&bytes).unwrap();
        assert_eq!(engine.invoke_json(&compiled, r#"{"value":21}"#).unwrap(), r#"{"value":21}"#);
        assert_eq!(compiled.source_sha256(), Sha256::digest(&bytes).as_slice());
    }

    #[test]
    fn rejects_oversized_components_before_compilation() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig {
            max_component_bytes: 8,
            ..ComponentEngineConfig::default()
        })
        .unwrap();
        assert!(matches!(
            engine.compile(&[0; 9]),
            Err(ComponentError::ComponentTooLarge { actual: 9, maximum: 8 })
        ));
    }

    #[test]
    fn precompiles_and_validates_a_host_specific_component() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let source = component(ECHO_COMPONENT);
        let artifact = engine.precompile(&source).unwrap();
        assert_eq!(artifact.component_abi_version, COMPONENT_ABI_VERSION);
        assert_eq!(Engine::detect_precompiled(&artifact.bytes), Some(Precompiled::Component));
        engine.validate_aot(&artifact, &source).unwrap();

        let mut tampered = artifact.clone();
        tampered.source_sha256[0] ^= 1;
        assert!(matches!(
            engine.validate_aot(&tampered, &source),
            Err(ComponentError::IncompatibleAot)
        ));
    }

    #[test]
    fn empty_linker_denies_unlinked_capability_imports() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let bytes = echo_component_with_import();
        let compiled = engine.compile(&bytes).unwrap();
        assert_eq!(compiled.required_imports()[0].to_string(), "tysel:test/host@0.4.0");
        assert!(matches!(engine.invoke_json(&compiled, "1"), Err(ComponentError::Instantiate(_))));
    }

    #[test]
    fn capability_imports_require_registry_trust_and_effective_grants() {
        use tysel_capability::CapabilityDescriptor;

        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let bytes = echo_component_with_import();
        let compiled = engine.compile(&bytes).unwrap();
        let registry = CapabilityRegistry::new([CapabilityDescriptor::new(
            "tysel:test/host@0.4.2".parse().unwrap(),
            [TrustMode::IsolatedTask],
        )
        .unwrap()])
        .unwrap();
        let grants = [CapabilityId("tysel:test".into())].into_iter().collect();

        let resolved =
            compiled.authorize_imports(&registry, TrustMode::IsolatedTask, &grants).unwrap();
        assert_eq!(resolved[0].to_string(), "tysel:test/host@0.4.2");
        assert!(compiled.authorize_imports(&registry, TrustMode::TrustedService, &grants).is_err());
        assert!(
            compiled
                .authorize_imports(&registry, TrustMode::IsolatedTask, &BTreeSet::new())
                .is_err()
        );
    }

    #[test]
    fn admitted_capability_provider_is_inserted_into_the_linker() {
        use tysel_capability::CapabilityDescriptor;

        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let compiled = engine.compile(&proxy_component_with_import()).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let audit_events = Arc::clone(&events);
        let provider = StringCapabilityProvider::new(
            CapabilityDescriptor::new(
                "tysel:test/host@0.4.2".parse().unwrap(),
                [TrustMode::IsolatedTask],
            )
            .unwrap(),
            "call",
            |input| Ok(format!(r#"{{"provider":{input}}}"#)),
        )
        .unwrap()
        .with_audit(move |result, _elapsed| {
            audit_events.lock().unwrap().push(result.to_owned());
        });
        let grants = [CapabilityId("tysel:test".into())].into_iter().collect();
        assert_eq!(
            engine
                .invoke_json_with_capabilities(
                    &compiled,
                    r#"{"linked":true}"#,
                    &[provider],
                    TrustMode::IsolatedTask,
                    &grants,
                )
                .unwrap(),
            r#"{"provider":{"linked":true}}"#
        );
        assert_eq!(*events.lock().unwrap(), ["ok"]);
    }

    #[test]
    fn capability_audit_observes_final_engine_limit_results() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let audit_events = Arc::clone(&events);
        let audit = move |result: &str, _elapsed: Duration| {
            audit_events.lock().unwrap().push(result.to_owned());
        };
        let input_handler_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_by_handler = Arc::clone(&input_handler_called);
        let input_handler = move |_input: &str| {
            called_by_handler.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok("unused".to_owned())
        };

        let input_error =
            invoke_string_capability(&input_handler, Some(&audit), "12345", 4, 10).unwrap_err();
        assert!(input_error.contains("input exceeds"));
        assert!(!input_handler_called.load(std::sync::atomic::Ordering::Relaxed));

        let output_error =
            invoke_string_capability(&|_| Ok("12345".to_owned()), Some(&audit), "1", 10, 4)
                .unwrap_err();
        assert!(output_error.contains("output exceeds"));
        assert_eq!(
            invoke_string_capability(&|_| Ok("1".to_owned()), Some(&audit), "1", 10, 10).unwrap(),
            "1"
        );
        assert_eq!(*events.lock().unwrap(), ["error", "error", "ok"]);
    }

    #[test]
    fn fuel_exhaustion_interrupts_guest_execution() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig {
            fuel: 1_000,
            ..ComponentEngineConfig::default()
        })
        .unwrap();
        let bytes = component(
            r#"
(component
  (core module $module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      global.get $heap)
    (func (export "run") (param i32 i32) (result i32)
      (loop $spin
        br $spin)
      unreachable))
  (core instance $instance (instantiate $module))
  (alias core export $instance "memory" (core memory $memory))
  (alias core export $instance "realloc" (core func $realloc))
  (alias core export $instance "run" (core func $run-core))
  (type $run-type
    (func (param "input" string) (result (result string (error string)))))
  (func $run (type $run-type)
    (canon lift (core func $run-core) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#,
        );
        let compiled = engine.compile(&bytes).unwrap();
        let error = engine.invoke_json(&compiled, "1").unwrap_err();
        assert!(matches!(error, ComponentError::Invoke(_)));
        assert!(error.to_string().contains("fuel"), "unexpected error: {error}");
    }

    #[test]
    fn validates_resource_configuration() {
        assert!(matches!(
            WasmComponentEngine::new(ComponentEngineConfig {
                fuel: 0,
                ..ComponentEngineConfig::default()
            }),
            Err(ComponentError::InvalidConfig(_))
        ));
        assert!(matches!(
            WasmComponentEngine::new(ComponentEngineConfig {
                max_execution_ms: 0,
                ..ComponentEngineConfig::default()
            }),
            Err(ComponentError::InvalidConfig(_))
        ));
    }

    #[test]
    fn epoch_deadline_bounds_guest_execution_time() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig {
            fuel: MAX_COMPONENT_FUEL,
            max_execution_ms: 10,
            ..ComponentEngineConfig::default()
        })
        .unwrap();
        let source = component(
            r#"
(component
  (core module $module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      global.get $heap)
    (func (export "run") (param i32 i32) (result i32)
      (loop $spin
        br $spin)
      unreachable))
  (core instance $instance (instantiate $module))
  (alias core export $instance "memory" (core memory $memory))
  (alias core export $instance "realloc" (core func $realloc))
  (alias core export $instance "run" (core func $run-core))
  (type $run-type
    (func (param "input" string) (result (result string (error string)))))
  (func $run (type $run-type)
    (canon lift (core func $run-core) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#,
        );
        let compiled = engine.compile(&source).unwrap();
        let error = engine.invoke_json(&compiled, "null").unwrap_err();
        assert!(matches!(error, ComponentError::Invoke(_)));
        assert!(error.to_string().contains("interrupt"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_wasi_interfaces_outside_the_language_runtime_profile() {
        let source = ECHO_COMPONENT.replacen(
            "(component",
            r#"(component
  (type $forbidden (instance))
  (import "wasi:sockets/network@0.2.0" (instance $network (type $forbidden)))"#,
            1,
        );
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        assert!(matches!(
            engine.compile(&component(&source)),
            Err(ComponentError::InvalidWasiRuntimeImport(name))
                if name == "wasi:sockets/network@0.2.0"
        ));
    }

    #[test]
    fn rejects_components_with_the_wrong_export_contract() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let source = component("(component)");
        assert!(matches!(engine.compile(&source), Err(ComponentError::Abi(_))));
    }

    #[test]
    fn rejects_oversized_input_before_instantiation() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig {
            max_input_bytes: 4,
            ..ComponentEngineConfig::default()
        })
        .unwrap();
        let compiled = engine.compile(&component(ECHO_COMPONENT)).unwrap();
        assert!(matches!(
            engine.invoke_json(&compiled, "12345"),
            Err(ComponentError::InputTooLarge { actual: 5, maximum: 4 })
        ));
    }

    #[test]
    fn rejects_invalid_json_at_both_sides_of_the_abi() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let compiled = engine.compile(&component(ECHO_COMPONENT)).unwrap();
        assert!(matches!(
            engine.invoke_json(&compiled, "not json"),
            Err(ComponentError::InvalidInputJson(_))
        ));

        let invalid_output_component = ECHO_COMPONENT
            .replacen(
                "(memory (export \"memory\") 1)",
                "(memory (export \"memory\") 1)\n    (data (i32.const 1024) \"not json\")",
                1,
            )
            .replacen("local.get $ptr\n      i32.store", "i32.const 1024\n      i32.store", 1)
            .replacen("local.get $len\n      i32.store", "i32.const 8\n      i32.store", 1);
        let compiled = engine.compile(&component(&invalid_output_component)).unwrap();
        assert!(matches!(
            engine.invoke_json(&compiled, "null"),
            Err(ComponentError::InvalidOutputJson(_))
        ));
    }

    #[test]
    fn rejects_oversized_output_after_guest_cleanup() {
        let engine = WasmComponentEngine::new(ComponentEngineConfig {
            max_output_bytes: 4,
            ..ComponentEngineConfig::default()
        })
        .unwrap();
        let compiled = engine.compile(&component(ECHO_COMPONENT)).unwrap();
        assert!(matches!(
            engine.invoke_json(&compiled, "12345"),
            Err(ComponentError::OutputTooLarge { actual: 5, maximum: 4 })
        ));
    }

    #[test]
    fn surfaces_the_guest_error_branch() {
        let error_component = ECHO_COMPONENT.replacen(
            "i32.const 0\n      i32.const 0\n      i32.store",
            "i32.const 0\n      i32.const 1\n      i32.store",
            1,
        );
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let compiled = engine.compile(&component(&error_component)).unwrap();
        assert!(matches!(
            engine.invoke_json(&compiled, r#""guest failure""#),
            Err(ComponentError::Guest(error)) if error == r#""guest failure""#
        ));
    }

    #[test]
    fn bounds_guest_error_payloads() {
        let error_component = ECHO_COMPONENT.replacen(
            "i32.const 0\n      i32.const 0\n      i32.store",
            "i32.const 0\n      i32.const 1\n      i32.store",
            1,
        );
        let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
        let compiled = engine.compile(&component(&error_component)).unwrap();
        let input = serde_json::to_string(&"好".repeat(MAX_COMPONENT_ERROR_BYTES)).unwrap();
        let error = engine.invoke_json(&compiled, &input).unwrap_err();
        assert!(
            matches!(error, ComponentError::Guest(message) if message.len() <= MAX_COMPONENT_ERROR_BYTES)
        );
    }
}
