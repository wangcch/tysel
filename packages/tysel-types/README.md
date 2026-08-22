# @tysel/types

Declaration-only public types for applications running on the Tysel native
runtime. Install this package as a development dependency and load it from an
application entry point:

```ts
import type {} from "@tysel/types";
```

The package declares the public `globalThis.tysel` API and exports application,
task, capability, durable-workflow, and WebSocket extension types. It contains
no runtime JavaScript and intentionally excludes underscored native bindings.
