# Capability matrix

Tysel denies undeclared resources. The isolated profile additionally denies host-facing capabilities even if the manifest lists them.

| Capability | Service | Isolated | Configuration |
| --- | --- | --- | --- |
| HTTP handler | Yes | Worker IPC | `[server]` |
| Outbound `fetch` | Allowlisted | No | `permissions.fetch` |
| Opaque secrets | Yes | Brokered | `permissions.secrets` |
| SQLite / Postgres / filesystem | Grant-bound | No | Corresponding permission/store |
| Inbound WebSocket | Yes | No | `server.websocket = true` |
| Outbound WebSocket | Allowlisted | No | `permissions.fetch` |
| LLM generation | Yes | No | Provider environment + secret |
| Durable handlers | Yes | Supervisor execution | `[durable]` |
| Timers, encoding, random | Yes | Yes | Limits only |

The service profile is process-trusted but capability-restricted. Linux isolated workers additionally apply Landlock, seccomp, and best-effort cgroup memory enforcement; macOS isolation is not the production security gate. See [ADR-008](../adr/008-wit-capability-abi.md) and [production operations](../operations/production.md).
