# Application module

An application default-exports a `TyselApp` object. It must declare at least one
HTTP, task-registry, or durable-registry entrypoint group; the selected command
or protocol still requires a concrete handler in that group.

Export a plain object and apply `satisfies TyselApp` (or
`TyselApp<TyselEnv>` when generated capability types are available). This is
the default and requires no runtime SDK import. Use `defineApp` only when one
property must contextually infer another, such as an MCP handler from its input
schema.

```ts
interface TyselAppMembers {
  fetch?: RuntimeFetchHandler;
  tasks?: Record<string, CronTask | QueueTask | McpTask>;
  durable?: Record<string, DurableHandler>;
}

type TyselApp = RequireAtLeastOne<TyselAppMembers>;
```

## HTTP handler

```ts
type FetchHandler = (request: Request) => Response | Promise<Response>;

type RuntimeFetchHandler = (
  request: Request,
  runtime: TyselRuntime,
) => Response | Promise<Response>;
```

The runtime calls `fetch` for inbound HTTP requests and injects the public
capability host as `runtime`. One-argument handlers remain valid, and
`globalThis.tysel` remains available for compatibility. The handler must return
a `Response` or a promise of one. Request and response bodies are bounded and
single-use according to the
[Request and Response contract](https://tysel.dev/reference/javascript/request/).

```ts
import type { TyselApp } from "@tysel/types";

export default {
  async fetch(request, runtime) {
    const url = new URL(request.url);
    return Response.json({
      method: request.method,
      path: url.pathname,
      isolateId: runtime.isolateId,
    });
  },
} satisfies TyselApp;
```

Prefer `satisfies TyselApp` over repeating parameter and return annotations. It
checks the complete default-export contract, contextually types each handler,
and preserves the application's precise inferred type for tests and adapters.
Use `TyselApp<TyselRuntimeWith<...>>` when a handler should expose only an
explicit subset of runtime capabilities.

## Request context

Non-HTTP task handlers receive:

```ts
interface RequestContext {
  requestId: string;
  deadlineMs: number;
}
```

`deadlineMs` is an absolute deadline in milliseconds, not a duration. Propagate
`requestId` into application logs when correlating work.

## Cron tasks

```ts
interface CronTask {
  kind: "cron";
  expression: string;
  handler(context: RequestContext): void | Promise<void>;
}
```

```ts
tasks: {
  cleanup: {
    kind: "cron",
    expression: "0 * * * *",
    async handler(context) {
      console.log("cleanup", context.requestId);
    },
  },
}
```

## Queue tasks

```ts
interface QueueTask {
  kind: "queue";
  name: string;
  handler(message: JsonValue, context: RequestContext): unknown | Promise<unknown>;
}
```

Messages and results must be JSON serializable. Use [`tysel queue`](../cli/tasks.md#tysel-queue)
to invoke a registered handler from the CLI.

## MCP tasks

MCP is the main case where `defineApp` adds value: it makes the literal `input`
schema the source of the handler parameter type. `tysel init --template mcp`
therefore adds the matching `@tysel/sdk` dependency.

```ts
import { defineApp } from "@tysel/sdk";

export default defineApp({
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

`defineApp()` keeps `input` as the single source of truth and contextually
infers the handler parameter. `InferMcpInput<Schema>` remains available for
advanced type composition, but ordinary handlers do not need to name it.
Handler input and result must be JSON serializable. TypeScript rejects schema
values outside the `McpInputType` vocabulary. Every declared property is
required by the current protocol adapter. [`tysel mcp`](../cli/tasks.md#tysel-mcp)
serves the registered tools over newline-delimited stdio.

For a single MCP task or a registry that mixes MCP with cron/queue tasks, the
optional `mcp({...})` constructor provides the same inference locally. It is a
convenience helper rather than the default documented application shape.

## Durable handlers

Durable handler values receive a `DurableContext` plus JSON input and return a
JSON value. Their replay rules are stricter than ordinary task handlers; see
the [Durable API](durable.md) before changing a deployed workflow.
