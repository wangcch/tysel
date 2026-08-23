# Manifest reference

Every application has one `tysel.toml` or `tysel.json`. Both formats use the
same schema and validation rules. Unknown fields are rejected.

## Root object

| Field | Required | Default | Reference |
| --- | --- | --- | --- |
| `schema_version` | No | `1` | Manifest schema version. Only version `1` is accepted. |
| `app` | Yes | — | [Application identity and entry](app-server.md#app). |
| `server` | No | Defaults applied | [Inbound server](app-server.md#server). |
| `permissions` | No | All lists empty | [Host capability grants](permissions.md). |
| `limits` | No | Defaults applied | [Application limits](limits.md). |
| `durable` | No | SQLite defaults | [Durable store](durable-observability.md#durable). |
| `observability` | No | JSON logs | [Telemetry](durable-observability.md#observability). |
| `tasks` | No | Empty map | [Project workflows](tasks.md). |

An omitted `schema_version` is accepted for older version-1 manifests. A newer
unsupported version fails with an upgrade diagnostic.

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

[tasks.verify]
description = "Check and test"
steps = [["check"], ["test"]]
```

## JSON form

The JSON representation has identical field names and values:

```json
{
  "schema_version": 1,
  "app": {
    "name": "orders",
    "entry": "src/index.ts",
    "profile": "service"
  },
  "server": {
    "listen": "127.0.0.1:3000",
    "http1": true,
    "http2": false,
    "websocket": false
  }
}
```

## Discovery and path resolution

Without `--manifest`, project commands search from the selected directory
upward. Keeping both formats in one directory is an error. The manifest
directory becomes the project root; `app.entry`, durable storage, and
filesystem roots resolve from it. A packaged executable has no external
manifest root, so runtime-relative storage resolves from its process working
directory.

Use the installed binary to inspect the exact schema and expanded values:

```sh
tysel config schema
tysel config show --format json
tysel config validate
```

See [Project and configuration commands](../cli/project.md) for conversion and
[Projects and configuration](../../concepts/projects-and-configuration.md) for
the discovery workflow.
