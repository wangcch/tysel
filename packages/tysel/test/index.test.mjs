import assert from "node:assert/strict";
import test from "node:test";

import { cron, defineApp, durableTask, mcp, queue } from "../src/index.ts";

test("defineApp preserves the application definition", () => {
  const app = { fetch: () => new Response("ok") };

  assert.equal(defineApp(app), app);
  assert.equal(defineApp()(app), app);
});

test("task constructors preserve configuration and handlers", async () => {
  const cronHandler = async () => {};
  const queueHandler = async (message) => message;
  const mcpHandler = async (input) => input.value;

  const cronTask = cron("0 * * * *", cronHandler);
  const queueTask = queue("events", queueHandler);
  const mcpTask = mcp({
    description: "echo",
    input: { value: "string" },
    handler: mcpHandler,
  });

  assert.deepEqual(cronTask, {
    kind: "cron",
    expression: "0 * * * *",
    handler: cronHandler,
  });
  assert.deepEqual(queueTask, {
    kind: "queue",
    name: "events",
    handler: queueHandler,
  });
  assert.deepEqual(mcpTask, {
    kind: "mcp",
    description: "echo",
    input: { value: "string" },
    handler: mcpHandler,
  });
  assert.equal(await queueTask.handler("payload", {}), "payload");
  assert.equal(await mcpTask.handler({ value: "payload" }, {}), "payload");
});

test("durableTask preserves the replay-safe entry point", async () => {
  const run = async (_ctx, input) => input + 1;
  const task = durableTask(run);

  assert.equal(task, run);
  assert.equal(await task({}, 41), 42);
});
