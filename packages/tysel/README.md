# tysel

Public typed helpers for defining a Tysel application. Applications remain
plain objects checked with `satisfies TyselApp`. Import `defineApp` only when a
definition needs cross-property inference, notably MCP input schemas.

```ts
import type { TyselApp } from "@tysel/types";

export default {
  async fetch() {
    return new Response("ok");
  },
} satisfies TyselApp;
```

The package exports `defineApp`, `cron`, `queue`, `mcp`, and `durableTask`, plus
the public application types from `@tysel/types`. Helpers return plain objects;
the native Tysel runtime provides the actual execution APIs.

`defineApp()` infers an MCP handler input from the literal input schema. The
package version must match the native toolchain and `@tysel/types` version.

```ts
import type { TyselEnv } from "./tysel-env.js";
import { defineApp } from "tysel";

export default defineApp<TyselEnv>()({
  tasks: {
    lookup: {
      kind: "mcp",
      description: "Look up a customer",
      input: { customerId: "string" },
      async handler(input) {
        return { customerId: input.customerId };
      },
    },
  },
});
```

For a single MCP task or a mixed task registry, `mcp({...})` provides the same
schema-driven inference locally. It is an optional convenience constructor,
not the default application style.

See the [runtime API](https://tysel.dev/reference/runtime/) and
[durable execution guide](../../docs/concepts/durable-execution.md).
