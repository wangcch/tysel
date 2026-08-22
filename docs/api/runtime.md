# Runtime API

## Application module

The default export can register an HTTP handler, scheduled/queue/MCP tasks,
and durable handlers:

```ts
export default {
  async fetch(request) { return new Response("ok"); },
  tasks: {
    cleanup: { kind: "cron", expression: "0 * * * *", async handler(ctx) {} },
    orders: { kind: "queue", name: "orders", async handler(input, ctx) {} },
    lookup: {
      kind: "mcp", description: "Look up an order", input: { id: "string" },
      async handler(input, ctx) { return { id: input.id }; },
    },
  },
  durable: {
    async workflow(ctx, input) { return await ctx.step("work", () => input); },
  },
};
```

Task handlers receive a context containing `requestId` and an absolute
`deadlineMs`. Fetch handlers receive a `Request`.

Applications export the object directly. `@tysel/types` supplies declarations
for editors and the TypeScript compiler:

```ts
import type { DurableContext, TyselApp } from "@tysel/types";
```

## Web and host APIs

Tysel provides `Request`, `Response`, `Headers`, allowlisted outbound `fetch`
and `WebSocket`, streams, UTF-8 encoders, timers, URL APIs, random UUID/bytes,
and Web Crypto digest/HMAC operations. The supported surface is documented in
the [Web API compatibility matrix](../architecture/javascript-runtime-compatibility.md).

| API | Contract | Grant |
| --- | --- | --- |
| `tysel.secrets.ref(name)` | Return an opaque `secret:name` handle, not the environment value. | `permissions.secrets` |
| `tysel.sqlite.exec/query` | Parameterized SQLite and bounded queries. | Trusted profile. |
| `tysel.postgres.exec/query` | Named, mode-limited Postgres. | `permissions.postgres` |
| `tysel.fs.read/write` | UTF-8 regular files beneath pinned roots. | `permissions.fs_read` / `fs_write` |
| `tysel.llm.generate(options)` | OpenAI-compatible bounded generation. | Provider configuration and declared secret. |
| `tysel.durable.start/sendSignal` | Start and wake durable handlers. | Durable store. |
| `tysel.acceptWebSocket()` | Accept the current inbound upgrade. | WebSocket enabled, trusted profile. |
| `new WebSocket(url)` | Connect, send text, and receive text/binary frames. `opened` exposes the connection promise; one outbound socket may be active per isolate request. | Trusted profile + `permissions.fetch` host. |

`crypto.subtle` supports SHA-256/384/512 `digest` plus raw HMAC `importKey`, `sign`, and `verify`. HMAC key usages are enforced and verification is performed by the native constant-time implementation.

## Server protocols

`server.http1` defaults to true and `server.http2` defaults to false. At least one must be enabled. Enabling both accepts HTTP/1.1 and cleartext HTTP/2 on the same listener; HTTP/2-only mode accepts h2c prior knowledge. WebSocket upgrades are HTTP/1.1-only. Terminate public TLS at an ingress or reverse proxy.

## Durable context

`step` and `effect` record replayable results; `sleep` and `waitForSignal`
persist suspension; `retry` records each attempt; `now` and `random` return
replay-safe values. Boundaries must be awaited sequentially. Inputs and results
must be JSON serializable and are limited to 1 MiB. Read the
[durable execution guide](../concepts/durable-execution.md) before changing a
handler with in-flight tasks.

## Errors

Unhandled HTTP errors return status 500 with `{ "error": { "code": "RUNTIME_ERROR", "message": "…", "requestId": "…" } }`. Oversized request bodies return 413 with code `BODY_TOO_LARGE`.

Private underscored host hooks are runtime implementation details. Applications
should use only Web globals, `globalThis.tysel`, and the exported package types.

For a complete workflow, choose a runnable project from the
[example gallery](../guides/examples.md). For exact configuration, continue to
the [manifest reference](../reference/manifest.md) and
[capability matrix](../capabilities/README.md).
