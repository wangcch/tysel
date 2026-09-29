import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const context = vm.createContext({ URL, TextEncoder, TextDecoder, __tysel_isReadableStream: () => false });
vm.runInContext(readFileSync(new URL("../web-api/source/http.js", import.meta.url), "utf8"), context);
const CustomRequest = vm.runInContext("Request", context);

test("Request redirect modes, copying and overrides match native Request", () => {
  for (const redirect of [undefined, "follow", "error", "manual", "", "invalid", null, 3]) {
    const run = Constructor => {
      try {
        const request = new Constructor("https://example.com", { redirect });
        return [request.redirect, request.clone().redirect, new Constructor(request).redirect,
          new Constructor(request, { redirect: "manual" }).redirect];
      } catch (error) { return [error.name]; }
    };
    assert.deepEqual(run(CustomRequest), run(Request));
  }
});

test("Request reads its redirect option only once", () => {
  for (const Constructor of [Request, CustomRequest]) {
    let reads = 0;
    const request = new Constructor("https://example.com", {
      get redirect() { reads++; return reads === 1 ? "error" : "follow"; },
    });
    assert.equal(request.redirect, "error");
    assert.equal(reads, 1);
  }
});
