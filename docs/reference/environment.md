# Environment variables

Environment variables configure the host and installation, not application
JavaScript. The runtime filters application access: declaring a secret makes
an opaque reference available through `tysel.secrets`, not through
`process.env`.

## Installation and toolchain

| Variable | Default | Scope |
| --- | --- | --- |
| `TYSEL_HOME` | `$HOME/.tysel` | Absolute root for a managed installation and its state. |
| `TYSEL_DOWNLOAD_BASE` | Official release endpoint | Trusted mirror or CI fixture used by install, doctor, and upgrade. |
| `TYSEL_STUB` | Located beside the CLI or in the build tree | Runtime stub used by `build`, `image`, and benchmark tooling. |
| `TYSEL_WORKER` | Located beside the CLI or in the build tree | Isolated worker executable override. |

`TYSEL_DOWNLOAD_BASE` changes the origin of executable artifacts. Set it only
to a controlled mirror with an equivalent verification policy. Prefer command
options such as `build --stub` when an override is specific to one invocation.

## Application capabilities

| Variable | Required when | Notes |
| --- | --- | --- |
| `TYSEL_POSTGRES_<NAME>` | The manifest grants named Postgres access | Connection URL for the one configured grant. Uppercase the name and replace `-` with `_`. |
| A name in `permissions.secrets` | A host capability needs that secret | The name and value are deployment-owned; JavaScript receives only a `SecretReference`. |

For a grant named `review-ro`, the host key is
`TYSEL_POSTGRES_REVIEW_RO`. Connection URLs and secret values must not be
placed in the manifest, command line, build artifact, or logs.

Under `tysel dev`, `.env` participates in local resolution, but only declared
secret names and the selected Postgres connection are imported. Unrelated
variables do not become application globals.

## Durable storage

| Variable | Precedence | Meaning |
| --- | ---: | --- |
| `TYSEL_DURABLE_POSTGRES_URL` | 1 | Select the Postgres durable store. A non-empty value wins. |
| `TYSEL_DURABLE_SQLITE_PATH` | 2 | Override the SQLite durable event-log path. |
| Manifest `[durable].path` directory | 3 | Places `durable-events.db` beside the application SQLite database when no host override is set. |

Use certificate-validated TLS for production Postgres and keep connection URLs
in a service manager or secret manager.

## LLM provider

| Variable | Default | Meaning |
| --- | --- | --- |
| `TYSEL_LLM_ENDPOINT` | Unset | Enable an OpenAI-compatible provider at this responses endpoint. |
| `TYSEL_LLM_MODEL` | Required with endpoint | Upstream provider model. |
| `TYSEL_LLM_ALIAS` | `default` | Application-facing model alias accepted by `generate`. |
| `TYSEL_LLM_SECRET` | `OPENAI_API_KEY` | Declared secret name used as provider credential. |

If `TYSEL_LLM_ENDPOINT` is unset or empty, the LLM capability is disabled.
The secret named by `TYSEL_LLM_SECRET` must also be present in
`permissions.secrets` and supplied by the host.

## OpenTelemetry

| Variable | Meaning |
| --- | --- |
| `OTEL_SDK_DISABLED=true` | Disable trace and metric export, even when endpoints remain set. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Shared OTLP/HTTP endpoint. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Trace-specific endpoint. |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Metric-specific endpoint. |

A signal-specific endpoint overrides or supplements the shared exporter
configuration according to the OpenTelemetry exporter. Manifest `traces` and
`metrics` fields are not yet propagated to packaged execution. Endpoints must be
HTTP(S), at most 2 KiB, and contain no userinfo, query, or fragment. Put
collector credentials in the standard OTLP headers configuration, never in the
URL.

## Advanced host integration

| Variable | Platform | Purpose |
| --- | --- | --- |
| `TYSEL_CGROUP` | Linux | cgroup v2 subtree used to attach isolated workers. |
| `TYSEL_COMPONENT_CAPABILITIES` | Component host | Comma-separated `tysel:fs`, `tysel:fs/read`, or `tysel:fs/write` deployment grants. Unknown values fail closed. |

These variables are deployment integration points, not application input.
Test-only variables and repository release-signing variables are intentionally
excluded from the application reference.

For a packaged Component the variable is unset by default, so application
capabilities remain denied even when the manifest requests them. See
[Component capabilities](component/capabilities.md) for the three-layer
intersection and local `tysel run` behavior.

See [Permissions](manifest/permissions.md), [Host capabilities](runtime/capabilities.md),
[Observability guide](../guides/observability.md), and
[Production operations](../operations/production.md).
