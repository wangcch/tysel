# Example gallery

Start a new application with one of the templates included in the installed
CLI. The template creates the project before you validate or run it:

| Template | Use it for |
| --- | --- |
| `http` | Fetch-style HTTP service; the recommended default. |
| `worker` | Service with one named Queue handler. |
| `mcp` | Isolated MCP stdio tool with validated input. |
| `minimal` | Smallest Fetch handler for a custom application structure. |

```sh
tysel init my-service --template http --yes
cd my-service
tysel task verify
tysel dev
```

Replace `http` with `worker`, `mcp`, or `minimal`. Run a Worker with
`tysel run`; start the MCP stdio transport with `tysel mcp`. See the
[`tysel init` reference](../reference/cli/project.md#tysel-init) for package,
manifest-format, test, and existing-project options.

The examples below are complete reference applications for capabilities beyond
the starter templates. Download the source archive matching the installed
Tysel release, then enter the example you want:

```sh
version="$(tysel --version | awk '{print $2}')"
curl -fsSL "https://github.com/wangcch/tysel/archive/refs/tags/v${version}.tar.gz" | tar -xz
cd "tysel-${version}/examples/hello-service"
```

Replace `hello-service` with another example directory from this page. This
downloads version-matched application source; it does not require a fork,
repository clone, Tysel build, or worker-path override. Wasm Component entries
instead use their version-matched starter bundles. Postgres, LLM, and MCP
examples require the extra environment documented in their README.

## Browse by capability

A **runnable** entry links to a complete source tree. A **reference recipe**
documents a supported API that does not yet have its own repository example;
do not treat a recipe as an end-to-end acceptance path.

The trusted `service` profile currently exposes SQLite, with
`durable.store = "sqlite"` and `./data/tysel.db` as manifest defaults. The
tables below therefore list SQLite as effective authority even when an example
does not call it. “No additional grants” would describe application intent,
not the runtime boundary; review `tysel inspect` for the effective deployment.

| Capability | Start here | Complete example | Coverage |
| --- | --- | --- | --- |
| HTTP and Fetch | [Application module](../reference/runtime/application.md#http-handler) | [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Runnable |
| Web-API npm router | [npm compatibility](../compatibility/README.md) | [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | Runnable; scan each dependency |
| WebSocket | [Service networking](service-networking.md) | [WebSocket service](https://github.com/wangcch/tysel/tree/main/examples/websocket-service) | Runnable |
| SQLite | [SQLite guide](sqlite.md) | [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Runnable |
| PostgreSQL | [PostgreSQL guide](postgresql.md) | [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Runnable |
| Filesystem | [Filesystem guide](filesystem.md) | [Filesystem transform](https://github.com/wangcch/tysel/tree/main/examples/filesystem-transform) | Runnable |
| Secrets | [Host capabilities](../reference/runtime/capabilities.md#secrets) | [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | Runnable as a brokered handle |
| LLM | [LLM gateway](llm-gateway.md) | [LLM service](https://github.com/wangcch/tysel/tree/main/examples/llm-service) / [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | Runnable with provider setup |
| Testing | [Testing API](../reference/runtime/testing.md) | — | Reference recipe |
| Cron and Queue | [Cron and Queue guide](cron-queue.md) | [Task worker](https://github.com/wangcch/tysel/tree/main/examples/task-worker) | Runnable |
| MCP | [Application module](../reference/runtime/application.md#mcp-tasks) | [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | Runnable |
| Durable execution | [Durable API](../reference/runtime/durable.md) | [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | Runnable |
| Wasm Component | [Component Reference](../reference/component/index.md) | [Rust echo](wasm-component-rust.md) / [Go echo](wasm-component-go.md) | Experimental runnable path |

Shell execution, child processes, FFI, and dynamic libraries are not missing
examples: they are intentionally outside the Tysel application contract. The
[capability matrix](../capabilities/README.md) records these denied surfaces.

## Start here

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Fetch handler and one-executable build | `tysel run` then `curl http://127.0.0.1:3000/hello` | `service`; SQLite default |
| [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | Web-API-first npm router | Install its Hono dependency, then run `tysel run` | `service`; SQLite default; run `tysel compat` |
| [WebSocket service](https://github.com/wangcch/tysel/tree/main/examples/websocket-service) | HTTP/1.1 upgrade and bounded text echo | `tysel run`, then connect to `ws://127.0.0.1:3000/ws` | `service`; inbound WebSocket; SQLite default |

## Tasks

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Task worker](https://github.com/wangcch/tysel/tree/main/examples/task-worker) | UTC Cron registration and named Queue consumer | `tysel queue jobs --input '{"id":"job_123","action":"reindex"}'` | `service`; SQLite default |

## Storage

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Persistent counter, runtime-relative file | `tysel run` then `curl 'http://127.0.0.1:3000/?key=visits'` | `service`; SQLite |
| [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Named grant; URL stays in the host | Set `TYSEL_POSTGRES_MAIN`, then run `tysel run` | `service`; `main:read-write`; SQLite default |
| [Filesystem transform](https://github.com/wangcch/tysel/tree/main/examples/filesystem-transform) | Separate pinned roots and bounded UTF-8 file transformation | `tysel run`, request `/transform`, then inspect `output/result.json` | `service`; read/write roots; SQLite default |

## Agents

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | LLM effect, suspend, restart, approval, single save | `./demo.sh` in the example directory after setting the LLM endpoint | `service`; LLM, secret, SQLite, durable store |
| [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | NDJSON MCP, validation, opaque secret handles | `tysel mcp` from the example directory | `isolated`; brokered secret only |
| [LLM service](https://github.com/wangcch/tysel/tree/main/examples/llm-service) | Host-configured provider alias, token usage, bounded HTTP adapter | Set the provider environment, then `tysel run` | `service`; provider secret; SQLite default |

## Isolation and Wasm

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Isolated plugin](https://github.com/wangcch/tysel/tree/main/examples/isolated-plugin) | Denied fetch/fs probes and worker replacement | `tysel run` from the example directory | `isolated`; host-facing capabilities remain denied |
| [Rust Component starter](wasm-component-rust.md) | One-shot JSON task, no ambient WASI | `printf '{"value":42}' \| tysel run` | `component`; experimental |
| [Go Component starter](wasm-component-go.md) | Same world with committed Go bindings | `printf '{"value":42}' \| tysel run` | `component`; experimental |

Review the [execution profile](../concepts/execution-profiles.md) and
[capability matrix](../capabilities/README.md) before adapting an example for
production.
