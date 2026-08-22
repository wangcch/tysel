# @tysel/types

Declaration-only public types for applications running on the Tysel native
runtime. After the first public package release, install it as a development
dependency:

```sh
pnpm add -D @tysel/types
```

From a source checkout before that release, build the package and add its local
directory instead.

Load the declarations from `tsconfig.json`:

```json
{
  "compilerOptions": {
    "types": ["@tysel/types"]
  }
}
```

or from an application module:

```ts
import type {} from "@tysel/types";

export default {
  async fetch(): Promise<Response> {
    const rows = await tysel.sqlite.query("SELECT 1 AS value");
    return Response.json({ rows });
  },
};
```

The package declares `globalThis.tysel` and the Tysel WebSocket extension. It
exports application, task, capability, and durable-execution types, including
`TyselApp`, `RequestContext`, `DurableContext`, and `TyselRuntime`.

It contains no runtime JavaScript. Installing it improves editor and compiler
feedback but does not polyfill Tysel APIs under Node.js. The package version
must match the native Tysel toolchain.

Underscored native bindings are intentionally private and are not declared.
See the [runtime API](../../docs/api/runtime.md).
