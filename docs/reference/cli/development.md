# Develop and test commands

All commands on this page accept `--manifest <file>` to select one manifest
instead of using upward discovery.

## Static validation

| Command | Options | Contract |
| --- | --- | --- |
| `tysel check` | `--manifest` | Validate the manifest, bundle, TypeScript, capability use, and Node builtin use. |
| `tysel types` | `--output`, `--check`, `--manifest` | Generate or verify a manifest-scoped TypeScript capability environment. |
| `tysel compat` | `--json`, `--strict`, `--deny-unknown`, `--manifest` | Classify imports and dependencies against the compatibility catalog. |
| `tysel inspect` | `--manifest` | Print effective capabilities and limits. |

`compat --strict` applies the command's strict compatibility policy;
`--deny-unknown` additionally treats unclassified dependencies as failures.
Use `--json` when a CI job needs a report rather than human text.

## `tysel types`

```text
tysel types [-o FILE] [--check] [--manifest FILE]
```

Generate `tysel-env.d.ts` from the effective manifest. The exported `TyselEnv`
contains core runtime utilities plus only the declared service capabilities.
Secret names become string literals, filesystem read/write methods are selected
independently, and read-only Postgres exposes only `query`.
`tysel init` generates this file for typed projects and wires the entry module
to `TyselApp<TyselEnv>` automatically.

Use the generated environment to narrow an application handler:

```ts
import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "../tysel-env.js";

export default {
  async fetch(_request, runtime) {
    return Response.json({ isolateId: runtime.isolateId });
  },
} satisfies TyselApp<TyselEnv>;
```

`--output` must remain relative to the project root. `--check` performs no write and exits non-zero when the declaration is missing
or stale. Run it in CI after changing manifest permissions. Runtime enforcement
remains authoritative; generated types do not grant capabilities. LLM provider
configuration currently lives outside the manifest and is therefore not added
to `TyselEnv` automatically. The global Web `fetch` declaration also cannot be
narrowed to manifest hostnames; runtime allowlist enforcement remains required.

## `tysel test`

```text
tysel test [PATHS]... [--timeout-ms MILLISECONDS] [--json] [--manifest FILE]
```

`PATHS` defaults to `tests/`. Each test file runs in a fresh QuickJS isolate.
The per-test timeout defaults to `5000` milliseconds. `--json` writes the test
report to stdout; failed assertions or timeouts exit non-zero.

```sh
tysel test
tysel test tests/http.test.ts --timeout-ms 10000 --json
```

See the [`@tysel/test` API](../runtime/testing.md) for test declarations and
assertions.

## `tysel dev`

```text
tysel dev [ENTRY] [--manifest FILE]
```

Bundle and serve the application, watch project files, and reload after a
successful rebuild. An explicit `ENTRY` overrides `app.entry` for this run.
Development mode can read `.env`, but only declared secrets and named Postgres
connections become visible through their host capabilities.

`tysel dev` rejects a `.wasm` Component entry. Components are one-shot tasks;
use `tysel run` and provide one JSON value on stdin.

## `tysel run`

```text
tysel run [ENTRY] [--manifest FILE]
```

Bundle and serve without file watching. An explicit `ENTRY` overrides
`app.entry` for this run. Runtime-relative paths resolve from the selected
project root.

For a `.wasm` Component, `run` does not start a server. It validates and
executes the Component once over bounded stdin/stdout, then exits. Local run
derives Component filesystem deployment authority from the checked manifest.
See [Wasm Components](../component/index.md).

For HTTP and process behavior, see [Application and server](../manifest/app-server.md)
and [Errors and machine output](../errors-and-output.md).
