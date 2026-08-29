# @tysel/test

Test helpers for applications running under `tysel test`. This package does not
provide a Node.js test runner.

`tysel init` adds the matching version automatically. For a manual installation:

```sh
version="$(tysel --version | awk '{print $2}')"
npm install --save-dev "@tysel/test@$version"
```

```ts
import app from "../src/index.ts";
import { assert, invokeFetch, test } from "@tysel/test";

test("returns a greeting", async () => {
  const response = await invokeFetch(app.fetch, "http://localhost/hello");
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { message: "hello" });
});
```

Run tests with `tysel test`. Each test file executes in a fresh QuickJS isolate.
`invokeFetch` calls a Fetch handler without opening a listener;
`invokeFetchWithRuntime` additionally injects a focused capability mock. The
package version must match the native toolchain.

See the [testing API](https://tysel.dev/reference/runtime/testing/).
