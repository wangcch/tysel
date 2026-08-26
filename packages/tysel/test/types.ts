import type { McpTask, TyselRuntimeWith } from "@tysel/types";

import { defineApp, mcp } from "../src/index.js";

const app = defineApp({
  tasks: {
    echo: {
      kind: "mcp",
      description: "echo",
      input: { value: "string", count: "integer" },
      async handler(input) {
        input.value.toUpperCase();
        input.count.toFixed();
        // @ts-expect-error the handler input follows the declared schema
        input.missing;
        return input.value.repeat(input.count);
      },
    },
  },
});

const echoResult = await app.tasks.echo.handler({ value: "ok", count: 2 });
echoResult.toUpperCase();
// @ts-expect-error defineApp preserves the handler's string output
echoResult.toFixed();

const legacyMcpTask: McpTask<{ value: string }> = {
  kind: "mcp",
  description: "legacy generic input",
  input: { value: "string" },
  async handler(input) {
    return input.value;
  },
};

const mixedApp = defineApp({
  tasks: {
    lookup: mcp({
      description: "lookup",
      input: { customerId: "string" },
      async handler(input) {
        return input.customerId.toUpperCase();
      },
    }),
    cleanup: {
      kind: "cron",
      expression: "0 * * * *",
      async handler(context) {
        context.requestId.toUpperCase();
      },
    },
  },
});

defineApp({
  tasks: {
    // @ts-expect-error a raw MCP task cannot bypass schema/handler correlation
    lookup: {
      kind: "mcp",
      description: "mismatched handler",
      input: { value: "number" },
      async handler(input: { value: string }) {
        return input.value;
      },
    },
  },
});

mcp({
  description: "invalid",
  input: { value: "number" },
  // @ts-expect-error the handler input is inferred from the literal schema
  async handler(input: { value: string }) {
    return input.value;
  },
});

const narrowApp = defineApp<TyselRuntimeWith<never>>()({
  async fetch(_request, runtime) {
    runtime.isolateId.toFixed();
    // @ts-expect-error unselected capabilities are unavailable
    await runtime.fs.read("input.txt");
    return new Response("ok");
  },
});

void app;
void legacyMcpTask;
void mixedApp;
void narrowApp;
