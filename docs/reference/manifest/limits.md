# Application limits

The optional `[limits]` table supplies application-level budgets. All fields
are non-negative integers; omitted fields receive the values below.

| Field | Default | Unit | Current runtime use |
| --- | ---: | --- | --- |
| `memory_mb` | `128` | MiB | QuickJS or isolated-worker memory budget. |
| `cpu_ms_per_turn` | `50` | ms | CPU budget for one JavaScript turn. |
| `request_timeout_ms` | `30000` | ms | Invocation deadline; also bounds configured LLM calls. |
| `max_in_flight` | `1000` | requests | Declared capacity target; not yet propagated into the packaged HTTP server. |
| `max_response_mb` | `16` | MiB | Declared response target; not yet propagated into the packaged HTTP server. |
| `max_request_mb` | `16` | MiB | Inbound request-body limit. |

```toml
[limits]
memory_mb = 256
cpu_ms_per_turn = 100
request_timeout_ms = 15000
max_in_flight = 200
max_response_mb = 8
max_request_mb = 4
```

The schema permits the full unsigned integer range for the corresponding
field (`u32` for memory, in-flight, and body limits; `u64` for CPU and request
timeouts). Platform or runtime ceilings may still reject or constrain values.

Schema acceptance and runtime enforcement are separate facts. In the current
package format, `memory_mb`, `cpu_ms_per_turn`, `request_timeout_ms`, and
`max_request_mb` are propagated to the runtime. `max_in_flight` and
`max_response_mb` remain visible through configuration inspection but require
deployment-level enforcement until the package/runtime boundary carries them.

Individual host capabilities also have fixed safety bounds, such as maximum
SQL, filesystem, LLM, protocol, and durable payload sizes. See
[Limits and defaults](../limits-and-defaults.md) for that inventory.

An oversized inbound request returns HTTP `413` with `BODY_TOO_LARGE`. Other
limit failures are reported through the relevant host API or the standard
runtime error envelope. A larger budget is not a substitute for backpressure,
bounded input validation, or deployment-level resource limits.
