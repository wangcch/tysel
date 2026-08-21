import assert from "node:assert/strict";
import test from "node:test";

import { bootstrap } from "../bootstrap/index.ts";
import { webApiVersion } from "../web-api/index.ts";

test("bootstrap completes without replacing host globals", () => {
  const originalFetch = globalThis.fetch;

  assert.equal(bootstrap(), undefined);
  assert.equal(globalThis.fetch, originalFetch);
});

test("web API version is a stable semantic version", () => {
  assert.match(webApiVersion, /^\d+\.\d+\.\d+$/);
  assert.equal(webApiVersion, "0.0.1");
});
