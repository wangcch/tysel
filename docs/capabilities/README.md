# Capability matrix

Tysel denies undeclared resources. A declaration is a request, not a guarantee:
the execution profile and deployment policy can reduce authority further. The
isolated profile denies host-facing capabilities even if the manifest lists
them.

| Capability | Service | Isolated | Component | Configuration |
| --- | --- | --- | --- | --- |
| HTTP handler | Yes | Worker IPC | No; one-shot task | `[server]` |
| Outbound `fetch` | Allowlisted | No | No | `permissions.fetch` |
| Opaque secret handles | Yes | Brokered handle only | No | `permissions.secrets` |
| SQLite / Postgres / Redis | Grant-bound | No | No | Corresponding permission/store |
| Filesystem | Grant-bound | No | Read/write WIT imports | Manifest roots + Component deployment policy |
| Inbound WebSocket | Yes | No | No | `server.websocket = true` |
| Outbound WebSocket | Allowlisted | No | No | `permissions.fetch` |
| LLM generation | Yes | No | No | Provider environment + secret |
| Durable handlers | Yes | Supervisor execution | No | `[durable]` |
| Timers, encoding, random | Yes | Yes | Restricted WASI clocks/random | Limits only |

The service profile is process-trusted but capability-restricted. Linux
isolated workers additionally apply Landlock, seccomp, resource limits, and
best-effort cgroup memory enforcement; macOS isolation is not the production
security gate.

The experimental Component profile uses `isolated-task` trust mode. Its only
implemented application capabilities are
[`tysel:fs/read@0.4.0` and `tysel:fs/write@0.4.0`](../reference/component/capabilities.md).
Actual imports, manifest roots, and deployment policy must all agree.

Use `tysel inspect` to see effective permissions and limits. See
[execution profiles](../concepts/execution-profiles.md),
[manifest reference](../reference/manifest/index.md), and
[production operations](../operations/production.md).
