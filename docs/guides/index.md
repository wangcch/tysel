# Guides

Guides are organized around outcomes. Use the reference section when you need
the complete contract for a command, manifest field, or runtime API.

## Start and develop

| Outcome | Guide | What you will verify |
| --- | --- | --- |
| Create, test, run, and package an HTTP service | [Getting started](../getting-started.md) | A request succeeds and the packaged executable runs without Node.js. |
| Add Tysel without replacing an existing Node project | [Create or adopt a project](../concepts/projects-and-configuration.md#create-or-adopt-a-project) | Existing files remain intact and Tysel uses its own compiler configuration. |
| Inspect or convert TOML and JSON configuration | [Inspect and convert configuration](../concepts/projects-and-configuration.md#inspect-and-convert-configuration) | Expanded defaults and converted output validate against one schema. |
| Add a reproducible project workflow | [Reproducible project tasks](../concepts/projects-and-configuration.md#reproducible-project-tasks) | `tysel task verify` runs bounded, shell-free steps. |

## Services, tasks, and agents

| Outcome | Start with | Required boundary |
| --- | --- | --- |
| Build a Fetch-style JSON API | [First service](../getting-started.md#write-a-fetch-handler) | `service` profile; no capability for a local response. |
| Use SQLite or Postgres | [Example gallery](examples.md#storage) | Trusted service plus the relevant database configuration. |
| Expose a function as an MCP tool | [MCP example](examples.md#tasks-and-agents) | Registered MCP task; isolated example keeps secret values in the host. |
| Run generated or third-party JavaScript | [Isolated plugin example](examples.md#isolation) | Linux is the production isolation security target. |
| Suspend for retry, time, or human approval | [Durable execution](../concepts/durable-execution.md) | Durable store and replay-safe boundaries. |
| Build a language-neutral one-shot task | [Rust Component](wasm-component-rust.md) or [Go Component](wasm-component-go.md) | Experimental `tysel:component/task@0.4.0`; restricted WASI. |

## Ship and operate

| Outcome | Guide | Important limit |
| --- | --- | --- |
| Build one executable | [Build one executable](../getting-started.md#build-one-executable) | The target must match the build host. |
| Generate a non-root container context | [Production deployment](../operations/production.md#deployment) | A Linux executable is required. |
| Diagnose, upgrade, or roll back the toolchain | [Installation lifecycle](../install.md#diagnose) | Managed operations apply to all three developer tools together. |
| Review production readiness | [Production operations](../operations/production.md) | Release evidence, platform scope, recovery, and monitoring are part of the gate. |

## How to use a guide

Run commands against the release you plan to deploy. Verify the result at each
boundary, keep manifest permissions minimal, and follow links into
[API reference](../reference/index.md) for defaults, errors, and unsupported
behavior. The [example gallery](examples.md) points to complete source trees
when a single snippet is not enough.
