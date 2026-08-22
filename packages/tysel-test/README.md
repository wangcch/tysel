# @tysel/test

Test helpers for applications running under `tysel test`.

```ts
import app from "../src/index.ts";
import { assert, invokeFetch, test } from "@tysel/test";

test("returns a greeting", async () => {
  const response = await invokeFetch(app.fetch, "http://localhost/hello");
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { message: "hello" });
});
```

`test(name, body)` registers synchronous or asynchronous tests. `assert`,
`assert.equal`, and `assert.deepEqual` report failures through the native test
runner. `invokeFetch` constructs a `Request` and invokes a Fetch handler without
opening a network listener.

Run the suite with:

```sh
tysel test
tysel test tests/http.test.ts --timeout-ms 10000
tysel test --json
```

Each test runs in a fresh QuickJS isolate. Test APIs are available only under
`tysel test`; importing this package does not provide a Node.js test runner.
The package version must match the installed Tysel native toolchain.
