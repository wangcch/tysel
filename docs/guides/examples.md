# Example gallery

The repository examples are runnable acceptance paths, not isolated snippets.
Each example includes its source, manifest, and exact verification commands.

## Start small

| Example | Demonstrates | Profile and capabilities |
| --- | --- | --- |
| [Hello service](https://github.com/wangcch/tysel/tree/main/examples/hello-service) | Minimal Fetch handler and one-executable build | `service`; none |
| [Hono API](https://github.com/wangcch/tysel/tree/main/examples/hono-api) | A Web-API-first npm dependency | `service`; package compatibility must still be checked |

## Storage

| Example | Demonstrates | Profile and capabilities |
| --- | --- | --- |
| [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker) | Persistent counter and runtime-relative storage | `service`; SQLite |
| [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service) | Named database grant with the connection URL kept in the host | `service`; `main:read-write` Postgres grant |

## Tasks and agents

| Example | Demonstrates | Profile and capabilities |
| --- | --- | --- |
| [Durable agent](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) | LLM effect, suspension, restart, human approval, and single persisted result | `service`; LLM, secret, SQLite, and durable store |
| [MCP tool](https://github.com/wangcch/tysel/tree/main/examples/mcp-tool) | MCP discovery, validation, bounded stdio, and opaque secret handles | `isolated`; brokered secret handle only |

## Isolation

| Example | Demonstrates | Profile and capabilities |
| --- | --- | --- |
| [Isolated plugin](https://github.com/wangcch/tysel/tree/main/examples/isolated-plugin) | Denied network/filesystem probes and worker crash recovery | `isolated`; host-facing capabilities remain denied |
| [Go Component](wasm-component-go.md) | A language-neutral task through the experimental Component ABI | `component`; experimental |
| [Rust Component](wasm-component-rust.md) | Rust guest component and restricted WASI contract | `component`; experimental |

## Run an example from a checkout

Build the developer tools once, then use `-C` so every command resolves paths
from the selected example project:

```sh
cargo build --locked \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service \
  -p tysel-isolate --bin tysel-worker
export PATH="$PWD/target/debug:$PATH"

tysel -C examples/hello-service config validate
tysel -C examples/hello-service task verify
tysel -C examples/hello-service run
```

Examples that use an isolated worker, external database, or LLM provider have
additional setup in their linked README. Review the
[execution profile](../concepts/execution-profiles.md) and
[capability matrix](../capabilities/README.md) before adapting an example for
production.
