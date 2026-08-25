# API reference

Reference pages describe exact interfaces: accepted values, defaults, side
effects, errors, and stability boundaries. Start with a
[guide](../guides/index.md) when you want a workflow rather than a contract.

The website index at `/reference` renders a flat module catalog. Use the sidebar
or the sections below to jump to a contract page.

## Runtime

| Module | Description |
| --- | --- |
| [Runtime](runtime/index.md) | Overview of exports and `globalThis.tysel` |
| [Core types](runtime/types.md) | `JsonValue`, profiles, trust modes |
| [Application module](runtime/application.md) | `fetch`, cron, queue, MCP exports |
| [Host capabilities](runtime/capabilities.md) | secrets, SQL, filesystem, LLM, WebSocket |
| [Durable API](runtime/durable.md) | effects, sleep, signals, resume |
| [Testing API](runtime/testing.md) | `test`, `assert`, `invokeFetch` |

## Web APIs

| Module | Description |
| --- | --- |
| [JavaScript APIs](javascript/index.md) | Index and compatibility inventory |
| [fetch](https://tysel.dev/reference/javascript/fetch) | Allowlisted outbound HTTP |
| [Request and Response](https://tysel.dev/reference/javascript/request) | Web-standard message types |
| [Headers](https://tysel.dev/reference/javascript/headers) | Case-insensitive header map |
| [URL](https://tysel.dev/reference/javascript/url) | URL and URLSearchParams |
| [WebSocket](https://tysel.dev/reference/javascript/websocket) | Client and server socket subset |
| [Crypto](https://tysel.dev/reference/javascript/crypto) | Web Crypto subset |
| [Timers](https://tysel.dev/reference/javascript/timers) | setTimeout and setInterval |
| [Event](https://tysel.dev/reference/javascript/event) | Event and EventTarget |
| [AbortController](https://tysel.dev/reference/javascript/abortcontroller) | AbortSignal for fetch and tasks |
| [TextEncoder](https://tysel.dev/reference/javascript/textencoder) | UTF-8 encode and decode |

## Wasm Components

| Module | Description |
| --- | --- |
| [Wasm Components](component/index.md) | Profile, trust mode, task boundary |
| [Component ABI](component/abi.md) | `tysel:component/task` world |
| [Runtime and WASI](component/runtime.md) | Portable execution limits and restricted WASI |
| [Component capabilities](component/capabilities.md) | Filesystem WIT imports |
| [Rust SDK](component/rust-sdk.md) | Guest types and dispatcher |
| [Go SDK](component/go-sdk.md) | Generated bindings |

## CLI

| Module | Description |
| --- | --- |
| [CLI](cli/index.md) | Global syntax and command map |
| [Project commands](cli/project.md) | `init`, `config`, schema |
| [Develop and test](cli/development.md) | `check`, `compat`, `test`, `dev`, `run`, `inspect` |
| [Tasks and protocols](cli/tasks.md) | `task`, `queue`, `mcp` |
| [Build and image](cli/delivery.md) | `build`, `image` |
| [Installation](cli/installation.md) | `doctor`, `upgrade` |
| [Evidence](cli/evidence.md) | `bench`, `release` |

## Manifest and host

| Module | Description |
| --- | --- |
| [Manifest](manifest/index.md) | Complete configuration field index, defaults, constraints, templates, and validation |
| [Application and server](manifest/app-server.md) | App identity and inbound server |
| [Permissions](manifest/permissions.md) | Network, secrets, SQL, filesystem |
| [Application limits](manifest/limits.md) | Body size, concurrency, timeouts |
| [Durable and observability](manifest/durable-observability.md) | Store, logs, telemetry |
| [Manifest tasks](manifest/tasks.md) | Project workflow definitions |
| [Environment variables](environment.md) | Installer, CLI, host adapters |
| [Limits and defaults](limits-and-defaults.md) | Hard bounds |
| [Errors and output](errors-and-output.md) | Exit codes and JSON envelopes |

## Related guides

| Topic | Link |
| --- | --- |
| Capability matrix | [docs/capabilities](../capabilities/README.md) |
| npm compatibility | [docs/compatibility](../compatibility/README.md) |
| Example gallery | [docs/guides/examples](../guides/examples.md) |

## Version-local truth

The reference describes the source tree that produced this documentation. For
an installed binary, its help and bundled schema are authoritative:

```sh
tysel --version
tysel --help
tysel config schema
tysel config show --format json
tysel inspect
```

Unknown manifest fields are errors. APIs or environment variables not listed
here should be treated as implementation details unless a release note promotes
them to a public contract.
