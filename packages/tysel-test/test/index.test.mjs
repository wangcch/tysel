import assert from "node:assert/strict";
import test from "node:test";

import { invokeFetch, invokeFetchWithRuntime } from "../src/index.ts";

test("invokeFetch constructs a Request from URL input", async () => {
  const response = await invokeFetch(
    async (request) =>
      Response.json({ method: request.method, body: await request.text() }),
    "https://example.test/items",
    { method: "POST", body: "payload" },
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { method: "POST", body: "payload" });
});

test("invokeFetch passes an existing Request through unchanged", async () => {
  const request = new Request("https://example.test/items");
  let received;

  const response = await invokeFetch((input) => {
    received = input;
    return new Response("ok", { status: 201 });
  }, request);

  assert.equal(received, request);
  assert.equal(response.status, 201);
  assert.equal(await response.text(), "ok");
});

test("invokeFetchWithRuntime injects an explicit capability host", async () => {
  const runtime = { isolateId: 42 };
  const response = await invokeFetchWithRuntime(
    (_request, injected) => Response.json({ isolateId: injected.isolateId }),
    runtime,
    "https://example.test/runtime",
  );

  assert.deepEqual(await response.json(), { isolateId: 42 });
});
