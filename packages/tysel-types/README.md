# @tysel/types

Declaration-only public types for applications running on the Tysel native
runtime. Install the version matching your Tysel toolchain as a development
dependency:

```sh
pnpm add -D @tysel/types
```

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
import type { TyselApp } from "@tysel/types";

export default {
  async fetch(_request, runtime) {
    const rows = await runtime.sqlite.query("SELECT 1 AS value");
    return Response.json({ rows });
  },
} satisfies TyselApp;
```

Using `satisfies TyselApp` validates the default export while preserving its
precise inferred type. It also contextually types handler parameters, so
applications do not need to repeat the `Request` and `Response` annotations.
An application must declare at least one HTTP, task-registry, or durable-registry
entrypoint group. MCP input descriptors are restricted to the protocol's
documented `McpInputType` vocabulary, and `InferMcpInput` derives a handler
parameter from a literal schema.

For ordinary MCP handlers, use the public `defineApp()` boundary from the
`tysel` package to infer that parameter automatically. `InferMcpInput` is
intended for advanced type composition and compatibility code.

The package declares `globalThis.tysel` and the Tysel WebSocket extension. It
exports application, task, capability, and durable-execution types, including
`TyselApp`, `RequestContext`, `DurableContext`, and `TyselRuntime`.

It contains no runtime JavaScript. Installing it improves editor and compiler
feedback but does not polyfill Tysel APIs under Node.js. The package version
must match the native Tysel toolchain.

Underscored native bindings are intentionally private and are not declared.
See the [runtime API](https://tysel.dev/reference/runtime/).
