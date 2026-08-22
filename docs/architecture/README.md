# Architecture

Tysel bundles a TypeScript application with a native runtime and a versioned TAP
trailer. At runtime, the service host owns capabilities and secrets, while JavaScript
executes with explicit limits. The isolated profile moves application execution into a
restricted worker process on Linux. Durable tasks use a persisted event log and leased
scheduler lifecycle so work can suspend and resume without replaying completed effects.

Accepted design decisions are recorded in the
[architecture decision records](../adr/index.md).
The main system boundaries are:

- Packaging: bundle, manifest, TAP trailer, and native service stub
- Execution: QuickJS isolates and optional Wasm Components
- Capabilities: network, secrets, storage, filesystem, and LLM host services
- Isolation: supervisor/worker IPC with Linux Landlock, seccomp, and cgroup controls
- Tasks: Cron, Queue, MCP, TaskRPC, and durable execution
- Operations: structured logs, OTLP export, release evidence, and recovery procedures

The [JavaScript runtime convergence contract](javascript-runtime-convergence.md)
defines ownership between `runtime-js`, QuickJS bindings, engine-neutral APIs,
and application packages.
The [JavaScript runtime compatibility matrix](javascript-runtime-compatibility.md)
lists the supported server-side Web API subset and explicit exclusions.

Production deployment and incident procedures are in the
[Production operations runbook](../operations/production.md).
