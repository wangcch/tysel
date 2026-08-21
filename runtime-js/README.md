# JavaScript runtime

`runtime-js` is the source of truth for JavaScript installed into every
QuickJS isolate. Rust owns native resources, policy, limits, and host functions;
these files own the JavaScript-facing Web and Tysel APIs.

The installation order is:

1. `web-api/runtime.js` installs Web-compatible globals, timers, encoding, and
   Web Crypto and AbortController wrappers.
2. Rust creates the private native hooks on `globalThis.tysel`.
3. `capability-client/runtime.js` installs fetch, WebSocket, storage,
   filesystem, secret, and LLM clients over those hooks.
4. `durable/control.js` installs durable task start/signal control.
5. When a replay session exists, `durable/runtime.js` replaces the durable
   object with replay-safe execution primitives.
6. Files in `bootstrap/` connect application exports to the engine's fetch,
   module-task, and durable-task entry points.

`tysel-engine-qjs` embeds these exact files with `include_str!`; do not copy
runtime JavaScript back into Rust string literals. Engine lifecycle, module
loading, and native result extraction remain in Rust.

`compatibility.json` is the versioned boundary between runtime JavaScript and
the engine build. Rust tests bind it to the TAP range and Component ABI exposed
by `tysel-package`, and to the selected QuickJS adapter identity. Update the
manifest intentionally whenever one of those contracts changes.

The three `runtime.js` files are deterministic generated artifacts. Edit the
files under each layer's `source/` directory and run:

```bash
pnpm --filter @tysel/runtime-js build:runtime
```

The normal `check` command runs the builder with `--check` and rejects stale or
manually edited embedded artifacts.

The TypeScript files describe build-time/public contracts. In particular,
`durable/index.ts` is reused by the `tysel` application SDK so the durable
context is defined once, while `capability-client/index.ts` is re-exported by
`@tysel/types` as the canonical host surface.

Fetch and streamed response reads expose cancellable native operation handles
internally. AbortSignal cancellation reaches the Reactor token and releases the
underlying request/body stream; the internal handles are not application APIs.

The URL and event implementations intentionally cover Tysel's tested subset of
the Web Platform rather than claiming full browser conformance. Extend authored
sources and QuickJS conformance tests together when adding supported behavior.
The authoritative feature inventory is `web-api/compatibility.json`; the
human-readable matrix lives in `docs/architecture/javascript-runtime-compatibility.md`.

Run the contract checks with:

```bash
pnpm --filter @tysel/runtime-js check
pnpm --filter @tysel/runtime-js test
cargo test -p tysel-engine-qjs --all-targets
```
