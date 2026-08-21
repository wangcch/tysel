# Getting started

## Create an application

```bash
tysel init hello-tysel
cd hello-tysel
tysel check
tysel compat
tysel test
tysel dev
```

`init` creates `src/index.ts`, a test under `tests/`, `tysel.toml`, TypeScript configuration, package scripts, and ignore rules. It checks every destination first, never overwrites an existing file, and rolls back files it created if a later write fails. Install dependencies once to activate the shared `@tysel/test` TypeScript contract.

`compat` reports four states: known compatible, shim required, unsupported, and unknown. CI can use `tysel compat --json --strict`, adding `--deny-unknown` when unreviewed dependencies must also fail.

## Write tests

Files ending in `.test.ts`, `.test.mts`, `.test.js`, or `.test.mjs` are discovered recursively under `tests/` by default.

```ts
test("returns a greeting", async () => {
  const response = await app.fetch(new Request("http://localhost/hello"));
  assert.equal(response.status, 200);
});
```

Each test runs in a fresh QuickJS isolate. Tests run in declaration order and may be asynchronous. The built-in API includes `assert(value)`, `assert.equal(actual, expected)`, and `assert.deepEqual(actual, expected)`. `--timeout-ms` is enforced by the engine for each test, including synchronous loops; a timed-out test does not prevent later tests from running. Any failure makes the command exit unsuccessfully.

## Build and containerize

```bash
tysel build --release
```

For containers, change `[server].listen` to `0.0.0.0:3000`. On Linux, `tysel image` builds the executable and image. On another host, pass an existing Linux ELF:

```bash
tysel image --tag hello-tysel:latest
tysel image --binary dist/linux/hello-tysel --context-only
```

The generated image runs as UID/GID `65532`. Supplied binaries must be little-endian ELF64 for x86_64 or AArch64 and either statically linked or use the glibc `ld-linux` interpreter expected by the default distroless base. Docker receives the matching `linux/amd64` or `linux/arm64` platform. Pin a release base with `--base-image registry/image@sha256:…`. Existing generated files are preserved unless `--force` is explicit.

## Machine-readable failures

```bash
tysel --error-format json check
tysel --error-format json test --json
```

Fatal CLI errors use `{ "error": { "code", "message" } }`. HTTP runtime failures add `requestId` and map JavaScript stack frames back to the current TypeScript source map. Test JSON is schema version 1 and includes mapped source stacks.
