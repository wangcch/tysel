# Guides

Guides are organized around outcomes. Use the reference section when you need
the complete contract for a command, manifest field, or runtime API.

## Start and develop

| Outcome | Guide | What you will verify |
| --- | --- | --- |
| Create, test, run, and package an HTTP service | [Getting started](../getting-started.md) | A request succeeds and the packaged executable runs without Node.js. |
| Add Tysel without replacing an existing Node project | [Create or adopt a project](../concepts/projects-and-configuration.md#create-or-adopt-a-project) | Existing files remain intact and Tysel uses its own compiler configuration. |
| Find or configure a `tysel.toml` field | [Complete manifest field index](../reference/manifest/index.md#complete-field-index) | Types, defaults, runtime status, cross-field rules, and starting configurations. |
| Inspect or convert TOML and JSON configuration | [Inspect and convert configuration](../concepts/projects-and-configuration.md#inspect-and-convert-configuration) | Expanded defaults and converted output validate against one schema. |
| Add a reproducible project workflow | [Reproducible project tasks](../concepts/projects-and-configuration.md#reproducible-project-tasks) | `tysel task verify` runs bounded, shell-free steps. |

## Services, tasks, and agents

| Outcome | Start with | Required boundary |
| --- | --- | --- |
| Build a Fetch-style JSON API | [First service](../getting-started.md#write-a-fetch-handler) | `service` profile; no capability for a local response. |
| Configure HTTP, h2c, WebSocket, and outbound hosts | [Service networking](service-networking.md) | Inbound listener and outbound allowlist are separate boundaries. |
| Size workers and reject overload predictably | [Concurrency and backpressure](concurrency-backpressure.md) | `503 OVERLOADED`; WebSockets retain admission permits. |
| Register UTC schedules and Queue consumers | [Cron and Queue](cron-queue.md) | JSON bounds, deadlines, idempotency, and bounded catch-up. |
| Route an LLM call without exposing credentials | [LLM gateway](llm-gateway.md) | One alias, host-owned secret, timeout, and audit bounds. |
| Transform files beneath pinned roots | [Filesystem](filesystem.md) | Separate UTF-8 read/write roots; traversal and symlinks denied. |
| Persist local application state | [SQLite](sqlite.md) | One runtime-owned connection and a stable deployment volume. |
| Connect to a remote relational database | [PostgreSQL](postgresql.md) | One named grant, host-injected URL, least-privilege database role. |
| Expose a function as an MCP tool | [MCP example](examples.md#agents) | Registered MCP task; isolated example keeps secret values in the host. |
| Run generated or third-party JavaScript | [Isolated plugin example](examples.md#isolation-and-wasm) | Linux is the production isolation security target. |
| Suspend for retry, time, or human approval | [Durable execution](../concepts/durable-execution.md) | Durable store and replay-safe boundaries. |
| Build a language-neutral one-shot task | [Rust Component](wasm-component-rust.md) or [Go Component](wasm-component-go.md) | Experimental `tysel:component/task@0.4.0`; restricted WASI. |

## Ship and operate

| Outcome | Guide | Important limit |
| --- | --- | --- |
| Build one executable | [Build one executable](../getting-started.md#build-one-executable) | The target must match the build host. |
| Generate and verify a non-root container | [Container image](container-image.md) | A matching Linux x64/arm64 executable is required. |
| Export structured logs, traces, and metrics | [Observability](observability.md) | OTLP environment variables are the active runtime control. |
| Correlate and safely map a production failure | [Debugging](debugging.md) | Packaged HTTP errors are not currently source-mapped. |
| Verify and sign an application artifact set | [Reproducible release](reproducible-release.md) | Evidence binds one build; it does not promise cross-host byte identity. |
| Diagnose, upgrade, or roll back the toolchain | [Installation lifecycle](../install.md#diagnose) | Managed operations apply to all three developer tools together. |
| Review production readiness | [Production operations](../operations/production.md) | Release evidence, platform scope, recovery, and monitoring are part of the gate. |

## How to use a guide

Run commands against the release you plan to deploy. Verify the result at each
boundary, keep manifest permissions minimal, and follow links into
[API reference](../reference/index.md) for defaults, errors, and unsupported
behavior. The [example gallery](examples.md) points to complete source trees
when a single snippet is not enough.
