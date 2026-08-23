# Application and server

## `[app]`

The required `app` table identifies the artifact, entry module, and execution
profile.

| Field | Required | Default | Accepted value |
| --- | --- | --- | --- |
| `name` | Yes | — | Letters, digits, `.`, `_`, and `-`; the first character must be a letter or digit. |
| `entry` | Yes | — | A project-relative TypeScript, JavaScript, or WebAssembly Component path. |
| `profile` | No | `service` | `service`, `isolated`, or `component`. |

`entry` cannot be absolute, traverse with `..`, use a Windows drive prefix, or
contain backslashes or control characters. The profile changes the trust and
host-capability boundary; it is not only an optimization preset.

```toml
[app]
name = "orders-api"
entry = "src/index.ts"
profile = "service"
```

See [Execution profiles](../../concepts/execution-profiles.md) and the
[Capability matrix](../../capabilities/README.md).

## `[server]`

The optional server table controls the inbound listener and protocols.

| Field | Default | Accepted value and behavior |
| --- | --- | --- |
| `listen` | `127.0.0.1:3000` | Socket address. Use `0.0.0.0` when a container must accept external traffic. |
| `http1` | `true` | Enable HTTP/1.1. |
| `http2` | `false` | Enable cleartext HTTP/2. |
| `websocket` | `false` | Permit inbound HTTP/1.1 WebSocket upgrades. |

At least one of `http1` and `http2` must be enabled. `websocket = true`
requires HTTP/1.1. When both HTTP versions are enabled, the listener accepts
HTTP/1.1 and h2c; HTTP/2-only mode requires h2c prior knowledge. Tysel does not
terminate public TLS, so deploy an ingress or reverse proxy for HTTPS.

```toml
[server]
listen = "0.0.0.0:3000"
http1 = true
http2 = true
websocket = true
```

Server body and concurrency settings live under [limits](limits.md), while
handler response and error contracts are documented under
[Errors and machine output](../errors-and-output.md).
