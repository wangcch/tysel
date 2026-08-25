# Example gallery

Start a new application with the templates included in the installed CLI:

```sh
tysel init my-service --yes
cd my-service
tysel task verify
tysel dev
```

The repository examples below are complete reference applications for
capabilities beyond the starter templates; each linked tree includes source, a
manifest, and its own README. Wasm Component entries instead link to
version-matched downloadable starter guides. After putting a project in your
workspace, run its commands from that directory with the published Tysel
installation; you do not need to build Tysel or set a worker path. Postgres,
LLM, and MCP examples require the extra environment documented in their README.

## Browse by capability

A **runnable** entry links to a complete source tree. A **reference recipe**
documents a supported API that does not yet have its own repository example;
do not treat a recipe as an end-to-end acceptance path.

| Capability | Start here | Complete example | Coverage |
| --- | --- | --- | --- |
| HTTP and Fetch | [Application module](../reference/runtime/application.md#http-handler) | [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Runnable |
| Web-API npm router | [npm compatibility](../compatibility/README.md) | [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | Runnable; scan each dependency |
| WebSocket | [Host capabilities](../reference/runtime/capabilities.md#websockets) | — | Reference recipe |
| SQLite | [Host capabilities](../reference/runtime/capabilities.md#sql) | [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Runnable |
| PostgreSQL | [Host capabilities](../reference/runtime/capabilities.md#sql) | [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Runnable |
| Filesystem | [Host capabilities](../reference/runtime/capabilities.md#filesystem) | — | Reference recipe |
| Secrets | [Host capabilities](../reference/runtime/capabilities.md#secrets) | [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | Runnable as a brokered handle |
| LLM | [Host capabilities](../reference/runtime/capabilities.md#llm-generation) | [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | Runnable with provider setup |
| Testing | [Testing API](../reference/runtime/testing.md) | — | Reference recipe |
| Cron and Queue | [Application module](../reference/runtime/application.md#cron-tasks) | — | Reference recipe |
| MCP | [Application module](../reference/runtime/application.md#mcp-tasks) | [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | Runnable |
| Durable execution | [Durable API](../reference/runtime/durable.md) | [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | Runnable |
| Wasm Component | [Component Reference](../reference/component/index.md) | [Rust echo](wasm-component-rust.md) / [Go echo](wasm-component-go.md) | Experimental runnable path |

Shell execution, child processes, FFI, and dynamic libraries are not missing
examples: they are intentionally outside the Tysel application contract. The
[capability matrix](../capabilities/README.md) records these denied surfaces.

## Start here

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Fetch handler and one-executable build | `tysel run` then `curl http://127.0.0.1:3000/hello` | `service`; no grants |
| [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | Web-API-first npm router | Install its Hono dependency, then run `tysel run` | `service`; run `tysel compat` |

## Storage

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Persistent counter, runtime-relative file | `tysel run` then `curl 'http://127.0.0.1:3000/?key=visits'` | `service`; SQLite |
| [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Named grant; URL stays in the host | Set `TYSEL_POSTGRES_MAIN`, then run `tysel run` | `service`; `main:read-write` |

## Agents

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | LLM effect, suspend, restart, approval, single save | `./demo.sh` in the example directory after setting the LLM endpoint | `service`; LLM, secret, SQLite, durable store |
| [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | NDJSON MCP, validation, opaque secret handles | `tysel mcp` from the example directory | `isolated`; brokered secret only |

## Isolation and Wasm

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Isolated plugin](https://github.com/wangcch/tysel/tree/main/examples/isolated-plugin) | Denied fetch/fs probes and worker replacement | `tysel run` from the example directory | `isolated`; host-facing capabilities remain denied |
| [Rust Component starter](wasm-component-rust.md) | One-shot JSON task, no ambient WASI | `printf '{"value":42}' \| tysel run` | `component`; experimental |
| [Go Component starter](wasm-component-go.md) | Same world with committed Go bindings | `printf '{"value":42}' \| tysel run` | `component`; experimental |

Review the [execution profile](../concepts/execution-profiles.md) and
[capability matrix](../capabilities/README.md) before adapting an example for
production.
