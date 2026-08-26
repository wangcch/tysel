use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use rquickjs::{Context, Ctx, Module, Runtime};
use serde::{Deserialize, Serialize};
use tysel_engine::{EngineError, InterruptReason, IsolateConfig, Value};
use tysel_task::TaskTrigger;

use crate::cpu::CpuBudget;
use crate::host;
use crate::isolate::{self, IsolateCancel};
use crate::queue;

pub const MAX_MODULE_TASKS: usize = 256;
pub const MAX_TASK_NAME_BYTES: usize = 128;
pub const MAX_TASK_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_TASK_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_TASK_RESULT_BYTES: usize = 1024 * 1024;
const MAX_MODULE_SOURCE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleTaskDefinition {
    pub name: String,
    #[serde(flatten)]
    pub kind: ModuleTaskKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMetadata {
    pub task_definitions: Vec<ModuleTaskDefinition>,
    pub durable_exports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModuleTaskKind {
    Cron { expression: String },
    Queue { queue: String },
    Mcp { description: String, input: BTreeMap<String, String> },
}

impl ModuleTaskDefinition {
    /// Convert registry metadata into the scheduler's unified trigger. Queue
    /// ingestion fills `message_id` when a concrete message is submitted.
    pub fn task_trigger(&self) -> TaskTrigger {
        match &self.kind {
            ModuleTaskKind::Cron { expression } => {
                TaskTrigger::Cron { name: self.name.clone(), expression: expression.clone() }
            }
            ModuleTaskKind::Queue { queue } => TaskTrigger::Queue {
                name: queue.clone(),
                handler: self.name.clone(),
                message_id: None,
            },
            ModuleTaskKind::Mcp { .. } => TaskTrigger::Mcp { tool: self.name.clone() },
        }
    }
}

enum TaskModuleOperation {
    Inspect,
    InspectDurable,
    Invoke { task_name: String, input_json: String, request_id: String, deadline_ms: u64 },
}

enum TaskModuleOutput {
    Definitions(Vec<ModuleTaskDefinition>),
    DurableExports(Vec<String>),
    Value(Value),
}

/// Evaluate a bundled application module and return its deterministic task
/// registry metadata. Handler functions never cross the isolate boundary.
pub fn inspect_task_module(
    source: &str,
    config: IsolateConfig,
) -> Result<Vec<ModuleTaskDefinition>, EngineError> {
    validate_source(source)?;
    match run_on_worker(source, config, TaskModuleOperation::Inspect)? {
        TaskModuleOutput::Definitions(definitions) => Ok(definitions),
        TaskModuleOutput::Value(_) | TaskModuleOutput::DurableExports(_) => unreachable!(),
    }
}

/// Names of durable functions exported as `default` or `default.durable`.
pub fn inspect_durable_exports(
    source: &str,
    config: IsolateConfig,
) -> Result<Vec<String>, EngineError> {
    validate_source(source)?;
    match run_on_worker(source, config, TaskModuleOperation::InspectDurable)? {
        TaskModuleOutput::DurableExports(names) => Ok(names),
        TaskModuleOutput::Definitions(_) | TaskModuleOutput::Value(_) => unreachable!(),
    }
}

/// Invoke one registered Cron, Queue, or MCP handler. Inputs and results must
/// be JSON and are bounded before and after isolate execution.
pub fn invoke_task_module(
    source: &str,
    task_name: &str,
    input_json: &str,
    request_id: &str,
    deadline_ms: u64,
    config: IsolateConfig,
) -> Result<Value, EngineError> {
    validate_source(source)?;
    validate_identifier("task name", task_name)?;
    validate_identifier("task request id", request_id)?;
    if input_json.len() > MAX_TASK_INPUT_BYTES {
        return Err(EngineError::Isolate(format!(
            "task input exceeds {MAX_TASK_INPUT_BYTES} bytes"
        )));
    }
    serde_json::from_str::<serde_json::Value>(input_json)
        .map_err(|error| EngineError::Isolate(format!("invalid task input: {error}")))?;
    if deadline_ms == 0 || deadline_ms > i64::MAX as u64 {
        return Err(EngineError::Isolate("task deadline is outside the supported range".into()));
    }
    let operation = TaskModuleOperation::Invoke {
        task_name: task_name.into(),
        input_json: input_json.into(),
        request_id: request_id.into(),
        deadline_ms,
    };
    match run_on_worker(source, config, operation)? {
        TaskModuleOutput::Value(value) => Ok(value),
        TaskModuleOutput::Definitions(_) | TaskModuleOutput::DurableExports(_) => unreachable!(),
    }
}

fn run_on_worker(
    source: &str,
    config: IsolateConfig,
    operation: TaskModuleOperation,
) -> Result<TaskModuleOutput, EngineError> {
    let source = source.to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tysel-qjs-task".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_task_module(&source, config, operation)
            }))
            .unwrap_or_else(|_| Err(EngineError::Isolate("quickjs task worker panicked".into())));
            let _ = sender.send(result);
        })
        .map_err(|error| EngineError::Isolate(error.to_string()))?;
    receiver.recv().map_err(|error| EngineError::Isolate(error.to_string()))?
}

fn run_task_module(
    source: &str,
    config: IsolateConfig,
    operation: TaskModuleOperation,
) -> Result<TaskModuleOutput, EngineError> {
    let cancel = IsolateCancel::new();
    let request_deadline = execution_deadline(&operation, config)?;
    let cpu = CpuBudget::new(Duration::from_millis(config.cpu_ms_per_turn.max(1)));
    let reactor = queue::spawn_reactor(cancel.flag(), request_deadline);
    let runtime = Runtime::new().map_err(isolate::js_err)?;
    runtime.set_memory_limit(config.memory_limit_bytes);
    {
        let cancel_flag = cancel.flag();
        let cpu = Arc::clone(&cpu);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            cancel_flag.load(std::sync::atomic::Ordering::SeqCst)
                || cpu.exhausted()
                || Instant::now() >= request_deadline
        })));
    }
    let context = Context::full(&runtime).map_err(isolate::js_err)?;
    context.with(|ctx| {
        host::install(ctx.clone(), reactor.io.clone(), 0).map_err(isolate::js_err)?;
        if let TaskModuleOperation::Invoke { task_name, input_json, request_id, deadline_ms } =
            &operation
        {
            ctx.globals().set("__tysel_task_name", task_name.as_str()).map_err(isolate::js_err)?;
            ctx.globals()
                .set("__tysel_task_input_json", input_json.as_str())
                .map_err(isolate::js_err)?;
            ctx.globals()
                .set("__tysel_task_request_id", request_id.as_str())
                .map_err(isolate::js_err)?;
            ctx.globals()
                .set("__tysel_task_deadline_ms", *deadline_ms as f64)
                .map_err(isolate::js_err)?;
        }
        Module::declare(ctx.clone(), "app.js", source).map_err(isolate::js_err)?;
        let boot = match operation {
            TaskModuleOperation::Inspect => BOOT_INSPECT,
            TaskModuleOperation::InspectDurable => BOOT_INSPECT_DURABLE,
            TaskModuleOperation::Invoke { .. } => BOOT_INVOKE,
        };
        let promise =
            Module::evaluate(ctx.clone(), "tysel-task-boot.js", boot).map_err(isolate::js_err)?;
        ctx.globals().set("__tysel_result", promise).map_err(isolate::js_err)
    })?;
    isolate::wait_until_settled(
        &runtime,
        &context,
        &reactor,
        &cancel,
        request_deadline,
        &cpu,
        None,
    )?;

    let output = context.with(|ctx| match operation {
        TaskModuleOperation::Inspect => {
            let json: String =
                ctx.globals().get("__tysel_task_manifest_json").map_err(isolate::js_err)?;
            decode_definitions(&json).map(TaskModuleOutput::Definitions)
        }
        TaskModuleOperation::InspectDurable => {
            let json: String =
                ctx.globals().get("__tysel_durable_exports_json").map_err(isolate::js_err)?;
            decode_durable_exports(&json).map(TaskModuleOutput::DurableExports)
        }
        TaskModuleOperation::Invoke { .. } => {
            let json: String =
                ctx.globals().get("__tysel_task_value_json").map_err(isolate::js_err)?;
            if json.len() > MAX_TASK_RESULT_BYTES {
                return Err(EngineError::Isolate(format!(
                    "task result exceeds {MAX_TASK_RESULT_BYTES} bytes"
                )));
            }
            let value = serde_json::from_str(&json)
                .map_err(|error| EngineError::Isolate(format!("invalid task result: {error}")))?;
            Ok(TaskModuleOutput::Value(isolate::from_json(value)))
        }
    });

    let _ = context.with(|ctx| {
        let _ = host::drop_host(&ctx);
        for name in [
            "__tysel_result",
            "__tysel_task_name",
            "__tysel_task_input_json",
            "__tysel_task_request_id",
            "__tysel_task_deadline_ms",
            "__tysel_task_manifest_json",
            "__tysel_durable_exports_json",
            "__tysel_task_value_json",
        ] {
            let _ = ctx.globals().remove(name);
        }
        Ok::<_, EngineError>(())
    });
    drop(context);
    runtime.set_interrupt_handler(None);
    runtime.run_gc();
    output
}

fn execution_deadline(
    operation: &TaskModuleOperation,
    config: IsolateConfig,
) -> Result<Instant, EngineError> {
    let config_budget = Duration::from_millis(config.request_timeout_ms.max(1));
    let TaskModuleOperation::Invoke { deadline_ms, .. } = operation else {
        return Ok(Instant::now() + config_budget);
    };
    let now_ms =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    if now_ms >= u128::from(*deadline_ms) {
        return Err(EngineError::Interrupted(InterruptReason::Timeout));
    }
    let remaining_ms = u64::try_from(u128::from(*deadline_ms) - now_ms).unwrap_or(u64::MAX);
    Ok(Instant::now() + config_budget.min(Duration::from_millis(remaining_ms)))
}

fn decode_definitions(json: &str) -> Result<Vec<ModuleTaskDefinition>, EngineError> {
    if json.len() > MAX_TASK_METADATA_BYTES {
        return Err(EngineError::Isolate(format!(
            "task metadata exceeds {MAX_TASK_METADATA_BYTES} bytes"
        )));
    }
    let definitions: Vec<ModuleTaskDefinition> = serde_json::from_str(json)
        .map_err(|error| EngineError::Isolate(format!("invalid task metadata: {error}")))?;
    if definitions.len() > MAX_MODULE_TASKS {
        return Err(EngineError::Isolate(format!(
            "task module defines more than {MAX_MODULE_TASKS} tasks"
        )));
    }
    for definition in &definitions {
        validate_identifier("task name", &definition.name)?;
        match &definition.kind {
            ModuleTaskKind::Cron { expression } => validate_field("cron expression", expression)?,
            ModuleTaskKind::Queue { queue } => validate_field("queue name", queue)?,
            ModuleTaskKind::Mcp { description, input } => {
                validate_field("MCP description", description)?;
                for (name, kind) in input {
                    validate_field("MCP input name", name)?;
                    validate_field("MCP input type", kind)?;
                }
            }
        }
    }
    Ok(definitions)
}

fn validate_source(source: &str) -> Result<(), EngineError> {
    if source.len() > MAX_MODULE_SOURCE_BYTES {
        return Err(EngineError::Module(format!(
            "task module exceeds {MAX_MODULE_SOURCE_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), EngineError> {
    if value.is_empty() || value.len() > MAX_TASK_NAME_BYTES {
        return Err(EngineError::Isolate(format!(
            "{label} must be 1..={MAX_TASK_NAME_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_field(label: &str, value: &str) -> Result<(), EngineError> {
    if value.is_empty() {
        return Err(EngineError::Isolate(format!("{label} must not be empty")));
    }
    Ok(())
}

fn decode_durable_exports(json: &str) -> Result<Vec<String>, EngineError> {
    if json.len() > MAX_TASK_METADATA_BYTES {
        return Err(EngineError::Isolate(format!(
            "durable export metadata exceeds {MAX_TASK_METADATA_BYTES} bytes"
        )));
    }
    let names: Vec<String> = serde_json::from_str(json)
        .map_err(|error| EngineError::Isolate(format!("invalid durable exports: {error}")))?;
    if names.len() > MAX_MODULE_TASKS {
        return Err(EngineError::Isolate(format!(
            "durable export count exceeds {MAX_MODULE_TASKS}"
        )));
    }
    Ok(names)
}

pub(crate) fn read_module_metadata(ctx: Ctx<'_>) -> Result<ModuleMetadata, EngineError> {
    let tasks_json: String =
        ctx.globals().get("__tysel_task_manifest_json").map_err(isolate::js_err)?;
    let durable_json: String =
        ctx.globals().get("__tysel_durable_exports_json").map_err(isolate::js_err)?;
    Ok(ModuleMetadata {
        task_definitions: decode_definitions(&tasks_json)?,
        durable_exports: decode_durable_exports(&durable_json)?,
    })
}

const BOOT_INSPECT: &str = include_str!("../../../runtime-js/bootstrap/task-inspect.js");

const BOOT_INSPECT_DURABLE: &str = include_str!("../../../runtime-js/bootstrap/durable-inspect.js");

const BOOT_INVOKE: &str = include_str!("../../../runtime-js/bootstrap/task-invoke.js");

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = r#"
export default {
  tasks: {
    nightly: {
      kind: "cron",
      expression: "0 0 * * *",
      async handler(ctx) { return { kind: "cron", requestId: ctx.requestId }; },
    },
    orders: {
      kind: "queue",
      name: "orders.created",
      async handler(message, ctx) {
        return { echoed: await tysel.echo(message.value), deadline: ctx.deadlineMs };
      },
    },
    lookup: {
      kind: "mcp",
      description: "Look up an order",
      input: { id: "string" },
      async handler(input) { return { id: input.id }; },
    },
  },
};
"#;

    #[test]
    fn discovers_sorted_cron_queue_and_mcp_tasks() {
        let definitions = inspect_task_module(MODULE, IsolateConfig::default()).unwrap();
        assert_eq!(definitions.len(), 3);
        assert_eq!(definitions[0].name, "lookup");
        assert!(matches!(definitions[0].kind, ModuleTaskKind::Mcp { .. }));
        assert_eq!(definitions[1].name, "nightly");
        assert_eq!(
            definitions[2],
            ModuleTaskDefinition {
                name: "orders".into(),
                kind: ModuleTaskKind::Queue { queue: "orders.created".into() },
            }
        );
        assert_eq!(
            definitions[1].task_trigger(),
            TaskTrigger::Cron { name: "nightly".into(), expression: "0 0 * * *".into() }
        );
        assert_eq!(
            definitions[2].task_trigger(),
            TaskTrigger::Queue {
                name: "orders.created".into(),
                handler: "orders".into(),
                message_id: None,
            }
        );
    }

    #[test]
    fn invokes_task_with_host_io_and_context() {
        let deadline_ms = unix_time_ms() + 10_000;
        let value = invoke_task_module(
            MODULE,
            "orders",
            r#"{"value":"ready"}"#,
            "task-42",
            deadline_ms,
            IsolateConfig::default(),
        )
        .unwrap();
        assert_eq!(
            value,
            Value::Record(vec![
                ("echoed".into(), Value::String("ready".into())),
                ("deadline".into(), Value::Number(deadline_ms as f64)),
            ])
        );
    }

    #[test]
    fn rejects_invalid_registry_input_and_expired_invocation() {
        let invalid = r#"export default { tasks: { broken: { kind: "queue", name: "q" } } };"#;
        assert!(inspect_task_module(invalid, IsolateConfig::default()).is_err());
        assert!(matches!(
            invoke_task_module(MODULE, "orders", "{}", "task-1", 1, IsolateConfig::default()),
            Err(EngineError::Interrupted(InterruptReason::Timeout))
        ));
    }

    fn unix_time_ms() -> u64 {
        u64::try_from(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis())
            .unwrap()
    }
}
