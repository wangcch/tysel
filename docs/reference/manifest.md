# Manifest reference

Every application has exactly one `tysel.toml` or `tysel.json`. Both formats
use the same schema and validation rules. A neighboring `package.json` is an
optional npm ecosystem sidecar and never overrides Tysel application settings.
CLI project commands resolve the application `entry`, runtime storage, and
filesystem paths from the discovered project root (the directory containing
the manifest). A packaged executable has no external manifest root and resolves
runtime-relative storage from its process working directory.

Without `--manifest`, project commands search from the selected directory
upward. Keeping both manifest formats in one directory is an error.

## Complete example

```toml
schema_version = 1

[app]
name = "orders"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:3000"
http1 = true
http2 = false
websocket = false

[permissions]
fetch = ["api.example.com"]
secrets = ["API_TOKEN"]
postgres = ["main:read-write"]
fs_read = ["./data/imports"]
fs_write = ["./data/exports"]

[limits]
memory_mb = 128
cpu_ms_per_turn = 50
request_timeout_ms = 30000
max_in_flight = 1000
max_response_mb = 16
max_request_mb = 16

[durable]
store = "sqlite"
path = "./data/tysel.db"

[observability]
logs = "json"
traces = "http://127.0.0.1:4318/v1/traces"
metrics = "http://127.0.0.1:4318/v1/metrics"
```

`schema_version` defaults to `1` for older manifests that omit it. An
unsupported newer version fails with an upgrade diagnostic.

## `[app]`

| Field | Required | Default | Description |
| --- | --- | --- | --- |
| `name` | Yes | — | Portable application and output name: letters, digits, `.`, `_`, and `-`; must start with a letter or digit. |
| `entry` | Yes | — | Project-relative TypeScript/JavaScript entry or `.wasm` Component; absolute paths and `..` are rejected. |
| `profile` | No | `service` | `service`, `isolated`, or `component`. |

## `[server]`

| Field | Default | Description |
| --- | --- | --- |
| `listen` | `127.0.0.1:3000` | Listener address. Use `0.0.0.0` in a container. |
| `http1` | `true` | Enable HTTP/1.1. |
| `http2` | `false` | Enable cleartext HTTP/2. |
| `websocket` | `false` | Permit inbound HTTP/1.1 WebSocket upgrades. |

At least one HTTP protocol must be enabled. WebSocket requires HTTP/1.1.
Terminate public TLS at an ingress or reverse proxy.

## `[permissions]`

All permission lists default to empty.

| Field | Values | Effect |
| --- | --- | --- |
| `fetch` | Hostnames | Allow outbound `fetch` and WebSocket connections. |
| `secrets` | Environment variable names | Allow opaque secret references. |
| `postgres` | Named grants | Allow the configured Postgres connection and mode. |
| `fs_read` | Directory roots | Allow UTF-8 regular-file reads below each root. |
| `fs_write` | Directory roots | Allow UTF-8 regular-file writes below each root. |

A Postgres grant is `name:read-write` or `name:read-only`; the mode defaults to
`read-write`. At most one grant is currently supported. Connection URLs do not
belong in the manifest. For a grant named `main`, set
`TYSEL_POSTGRES_MAIN` in the host environment.

Filesystem roots must be non-empty and unique. Each list accepts at most 64
roots.

## `[limits]`

| Field | Default | Description |
| --- | --- | --- |
| `memory_mb` | `128` | Application memory budget. |
| `cpu_ms_per_turn` | `50` | CPU budget for one JavaScript turn. |
| `request_timeout_ms` | `30000` | End-to-end invocation deadline. |
| `max_in_flight` | `1000` | Concurrent invocation limit. |
| `max_response_mb` | `16` | Maximum response body size. |
| `max_request_mb` | `16` | Maximum request body size. |

Set limits from measured workload behavior. A larger limit is not a substitute
for backpressure or bounded input validation.

## `[durable]`

| Field | Default | Description |
| --- | --- | --- |
| `store` | `sqlite` | Durable event store implementation. |
| `path` | `./data/tysel.db` | SQLite database path. |

## `[observability]`

| Field | Default | Description |
| --- | --- | --- |
| `logs` | `json` | Structured log format. |
| `traces` | unset | OTLP/HTTP trace endpoint. |
| `metrics` | unset | OTLP/HTTP metric endpoint. |

Use `tysel check` to validate a manifest and `tysel inspect` to print the
effective capabilities and limits.

## `[tasks]`

Tasks compose Tysel commands without requiring `package.json` or a platform
shell. A task can depend on other tasks and contain ordered argv steps:

```toml
[tasks.verify]
description = "Check and test"
steps = [
  ["check"],
  ["test"],
]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
```

Dependencies run once in dependency order. Cycles, missing dependencies,
empty steps, duplicate dependencies, and unsupported commands are rejected
while loading the manifest. Steps do not invoke a shell and cannot call
`init`, `upgrade`, `release`, `doctor`, `bench`, `config`, or another task.

```sh
tysel task --list
tysel task verify
```
