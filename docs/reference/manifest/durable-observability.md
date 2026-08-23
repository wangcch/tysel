# Durable storage and observability

## `[durable]`

The optional durable table selects the event store used by durable handlers.

| Field | Default | Current implementation |
| --- | --- | --- |
| `store` | `sqlite` | `sqlite` enables the application SQLite capability. Other strings are schema-valid but do not configure a manifest-backed store. |
| `path` | `./data/tysel.db` | Runtime-relative application SQLite database path. |

```toml
[durable]
store = "sqlite"
path = "./data/workflows.db"
```

In project commands, a relative path resolves from the manifest directory. In
a packaged executable, it resolves from the process working directory. When a
module exports durable handlers, the default durable event log is named
`durable-events.db` beside this application database. Set
`TYSEL_DURABLE_SQLITE_PATH` to choose the durable event-log file explicitly.

The runtime can also use a Postgres-backed durable store when configured by the
host with `TYSEL_DURABLE_POSTGRES_URL`. This is host configuration, not a
version-1 manifest `store` value.

See [Durable API](../runtime/durable.md), [Durable execution](../../concepts/durable-execution.md),
and [Production operations](../../operations/production.md) for replay and
backup requirements.

## `[observability]`

| Field | Default | Current implementation |
| --- | --- | --- |
| `logs` | `json` | Case-insensitive `json` enables structured runtime logs; other strings disable that JSON logger. |
| `traces` | Unset | Nullable endpoint intent recorded by the manifest schema. |
| `metrics` | Unset | Nullable endpoint intent recorded by the manifest schema. |

```toml
[observability]
logs = "json"
traces = "http://otel-collector:4318/v1/traces"
metrics = "http://otel-collector:4318/v1/metrics"
```

The current package manifest carries `logs` but does not yet propagate
`traces` or `metrics` into the packaged runtime. Configure active export with
the standard OpenTelemetry environment variables. This distinction is
intentional in the reference: the fields are schema-valid, but environment
configuration is the current operational control. Do not place authentication
tokens in the manifest; use secret-bearing deployment configuration.

See [Environment variables](../environment.md) for supported OpenTelemetry
controls and [Production operations](../../operations/production.md) for
collector and monitoring guidance.
