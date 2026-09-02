# @tysel/sdk

Typed definition helpers for Tysel applications. This npm package is not the
Tysel CLI or native runtime; install those through the
[Tysel installer](https://tysel.dev/docs/install/).

`tysel init` adds the matching package version automatically. For a manual
installation, pin it to the native toolchain version:

```sh
version="$(tysel --version | awk '{print $2}')"
npm install --save-dev "@tysel/sdk@$version" "@tysel/types@$version"
```

Use `defineApp` when an application needs cross-property inference, notably for
MCP input schemas:

```ts
import type { TyselEnv } from "./tysel-env.js";
import { defineApp } from "@tysel/sdk";

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

The package also exports `cron`, `queue`, `mcp`, and `durableTask`, plus public
types from `@tysel/types`. Helpers return plain objects; the native runtime
provides execution APIs. Applications that do not need helper inference can use
`satisfies TyselApp` directly.

See the [runtime API](https://tysel.dev/reference/runtime/) and
[durable execution guide](https://tysel.dev/docs/concepts/durable-execution/).
