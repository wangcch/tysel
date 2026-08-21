# Runtime API

## Application module

The default export can register an HTTP handler, scheduled/queue/MCP tasks, and durable handlers:

```ts
export default {
  async fetch(request, ctx) { return new Response("ok"); },
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

Request/task context exposes `requestId` and absolute `deadlineMs`.

## Web and host APIs

Tysel provides `Request`, `Response`, `Headers`, allowlisted outbound `fetch`, UTF-8 encoders, timers, URL APIs, random UUID/bytes, and Web Crypto digest/HMAC operations.

| API | Contract | Grant |
| --- | --- | --- |
| `tysel.secrets.ref(name)` | Opaque `secret:name` handle. | `permissions.secrets` |
| `tysel.sqlite.exec/query` | Parameterized SQLite and bounded queries. | Trusted profile. |
| `tysel.postgres.exec/query` | Named, mode-limited Postgres. | `permissions.postgres` |
| `tysel.fs.read/write` | UTF-8 regular files beneath pinned roots. | `permissions.fs_read` / `fs_write` |
| `tysel.llm.generate(options)` | OpenAI-compatible bounded generation. | Provider configuration. |
| `tysel.durable.start/sendSignal` | Start and wake durable handlers. | Durable store. |
| `tysel.acceptWebSocket()` | Accept the current inbound upgrade. | WebSocket enabled, trusted profile. |

## Durable context

`step` and `effect` record replayable results; `sleep` and `waitForSignal` persist suspension; `retry` records each attempt; `now` and `random` return replay-safe values. Boundaries must be awaited sequentially. Inputs and results must be JSON serializable and are limited to 1 MiB.

## Errors

Unhandled HTTP errors return status 500 with `{ "error": { "code": "RUNTIME_ERROR", "message": "…", "requestId": "…" } }`. Oversized request bodies return 413 with code `BODY_TOO_LARGE`.
