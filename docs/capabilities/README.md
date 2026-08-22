# Capability matrix

Tysel denies undeclared resources. A declaration is a request, not a guarantee:
the execution profile and deployment policy can reduce authority further. The
isolated profile denies host-facing capabilities even if the manifest lists
them.

| Capability | Service | Isolated | Configuration |
| --- | --- | --- | --- |
| HTTP handler | Yes | Worker IPC | `[server]` |
| Outbound `fetch` | Allowlisted | No | `permissions.fetch` |
| Opaque secret handles | Yes | Brokered handle only | `permissions.secrets` |
| SQLite / Postgres / filesystem | Grant-bound | No | Corresponding permission/store |
| Inbound WebSocket | Yes | No | `server.websocket = true` |
| Outbound WebSocket | Allowlisted | No | `permissions.fetch` |
| LLM generation | Yes | No | Provider environment + secret |
| Durable handlers | Yes | Supervisor execution | `[durable]` |
| Timers, encoding, random | Yes | Yes | Limits only |

The service profile is process-trusted but capability-restricted. Linux
isolated workers additionally apply Landlock, seccomp, resource limits, and
best-effort cgroup memory enforcement; macOS isolation is not the production
security gate.

Use `tysel inspect` to see effective permissions and limits. See
[execution profiles](../concepts/execution-profiles.md),
[manifest reference](../reference/manifest.md), and
[production operations](../operations/production.md).
