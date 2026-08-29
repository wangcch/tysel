# @tysel/types

Declaration-only types for applications running on the Tysel native runtime.
This package improves editor and compiler feedback; it does not polyfill Tysel
APIs under Node.js.

`tysel init` adds the matching version automatically. For a manual installation:

```sh
version="$(tysel --version | awk '{print $2}')"
npm install --save-dev "@tysel/types@$version"
```

Use the manifest-generated environment to narrow handlers to their declared
capabilities:

```ts
import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "./tysel-env.js";

export default {
  async fetch(_request, runtime) {
    const rows = await runtime.sqlite.query("SELECT 1 AS value");
    return Response.json({ rows });
  },
} satisfies TyselApp<TyselEnv>;
```

Run `tysel types` after changing manifest permissions. The package exports
application, task, capability, WebSocket, and durable-execution types, including
`TyselApp`, `RequestContext`, `DurableContext`, and `TyselRuntime`. It also
declares `globalThis.tysel`; underscored native bindings remain private.

The package version must match the native toolchain. See the
[runtime API](https://tysel.dev/reference/runtime/).
