# Runtime reference

Tysel applications export handlers and use browser-style globals plus the
`globalThis.tysel` host object. `@tysel/types` provides the public declarations
for editors and static checks; it does not install a JavaScript polyfill.

```ts
import type { TyselApp } from "@tysel/types";

const app: TyselApp = {
  async fetch(request) {
    return new Response(`received ${request.method}`);
  },
};

export default app;
```

## Surface map

| Surface | Reference |
| --- | --- |
| JSON values, profiles, trust modes, and shared value types | [Core types](types.md) |
| Default export, HTTP handler, cron, queue, and MCP tasks | [Application module](application.md) |
| Secrets, SQLite, Postgres, filesystem, LLM, and WebSocket APIs | [Host capabilities](capabilities.md) |
| Replay-safe workflows and signals | [Durable API](durable.md) |
| Test declaration, assertions, and handler invocation | [Testing API](testing.md) |
| `Request`, `Response`, `Headers`, `fetch`, crypto, timers, and other globals | [JavaScript API reference](../javascript/index.md) |

## Stability boundary

Public application code should use only:

- the default application export described here;
- Web globals listed in the compatibility inventory;
- `globalThis.tysel` members declared by `@tysel/types`;
- exports documented for `@tysel/test` in test modules.

Underscored host hooks and imports from internal runtime files are not public
contracts. A declaration in `@tysel/types` describes the accepted TypeScript
shape; availability still depends on the execution profile, manifest grants,
and host configuration.

See the [Capability matrix](../../capabilities/README.md) for that three-part
availability check and [Limits and defaults](../limits-and-defaults.md) for
fixed payload bounds.
