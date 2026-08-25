# Example gallery

The repository examples are runnable acceptance paths, not isolated snippets.
Each tree includes source, a manifest, and the exact commands used to verify
it. The website catalog at `/examples` presents the same set as a scanable
index; this page is the documentation source of truth.

## Run from a checkout

Build the developer tools once, then use `-C` so every command resolves paths
from the selected example:

```sh
cargo build --locked \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service \
  -p tysel-isolate --bin tysel-worker
export PATH="$PWD/target/debug:$PATH"

tysel -C examples/hello-service config validate
tysel -C examples/hello-service check
tysel -C examples/hello-service run
```

Isolated workers need `TYSEL_WORKER` set to the absolute `tysel-worker` path.
Postgres, LLM, and MCP examples have extra environment in their README.

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
| [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Fetch handler and one-executable build | `tysel -C examples/hello-service run` then `curl http://127.0.0.1:3000/hello` | `service`; no grants |
| [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | Web-API-first npm router | `pnpm install` then `tysel -C examples/hono-api run` | `service`; run `tysel compat` |

## Storage

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Persistent counter, runtime-relative file | `tysel -C examples/sqlite-worker run` then `curl 'http://127.0.0.1:3000/?key=visits'` | `service`; SQLite |
| [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Named grant; URL stays in the host | `export TYSEL_POSTGRES_MAIN=…` then `tysel -C examples/postgres-service run` | `service`; `main:read-write` |

## Agents

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | LLM effect, suspend, restart, approval, single save | `./demo.sh` in the example directory after setting the LLM endpoint | `service`; LLM, secret, SQLite, durable store |
| [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | NDJSON MCP, validation, opaque secret handles | `export TYSEL_WORKER=…` then `tysel mcp` from the example directory | `isolated`; brokered secret only |

## Isolation and Wasm

| Example | Demonstrates | Try | Profile |
| --- | --- | --- | --- |
| [Isolated plugin](https://github.com/wangcch/tysel/tree/main/examples/isolated-plugin) | Denied fetch/fs probes and worker replacement | `export TYSEL_WORKER=…` then `tysel -C examples/isolated-plugin run` | `isolated`; host-facing capabilities remain denied |
| [Rust Component](https://github.com/wangcch/tysel/tree/main/sdk/examples/rust-echo) | One-shot JSON task, no ambient WASI | `cd sdk/examples/rust-echo && printf '{"value":42}' \| tysel run` | `component`; experimental |
| [Go Component](https://github.com/wangcch/tysel/tree/main/sdk/examples/go-echo) | Same world with committed Go bindings | `cd sdk/examples/go-echo && printf '{"value":42}' \| tysel run` | `component`; experimental |

Review the [execution profile](../concepts/execution-profiles.md) and
[capability matrix](../capabilities/README.md) before adapting an example for
production.
