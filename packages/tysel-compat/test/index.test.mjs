import assert from "node:assert/strict";
import test from "node:test";

import { shimAllowlist } from "../src/index.ts";

test("compatibility shim allowlist remains explicit and duplicate-free", () => {
  assert.deepEqual(shimAllowlist, [
    "buffer",
    "path",
    "util",
    "events",
    "assert",
    "querystring",
  ]);
  assert.equal(new Set(shimAllowlist).size, shimAllowlist.length);
});
