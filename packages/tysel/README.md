# tysel

Internal typed helpers for defining a Tysel application in this workspace.

```ts
import { defineApp, queue } from "tysel";

export default defineApp({
  async fetch() {
    return new Response("ok");
  },
  tasks: {
    emails: queue("emails", async (message: { to: string }) => {
      return { accepted: message.to };
    }),
  },
});
```

The package exports `defineApp`, `cron`, `queue`, `mcp`, and `durableTask`, plus
the public application types from `@tysel/types`. Helpers return plain objects;
the native Tysel runtime provides the actual execution APIs.

This package is private and is not part of the current application installation
path. Public applications should export a conforming object directly and use
`@tysel/types` for declarations. If this package is published later, its
version will need to match the native toolchain.

See the [runtime API](../../docs/api/runtime.md) and
[durable execution guide](../../docs/concepts/durable-execution.md).
