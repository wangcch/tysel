# Develop and test commands

All commands on this page accept `--manifest <file>` to select one manifest
instead of using upward discovery.

## Static validation

| Command | Options | Contract |
| --- | --- | --- |
| `tysel check` | `--manifest` | Validate the manifest, bundle, TypeScript, capability use, and Node builtin use. |
| `tysel compat` | `--json`, `--strict`, `--deny-unknown`, `--manifest` | Classify imports and dependencies against the compatibility catalog. |
| `tysel inspect` | `--manifest` | Print effective capabilities and limits. |

`compat --strict` applies the command's strict compatibility policy;
`--deny-unknown` additionally treats unclassified dependencies as failures.
Use `--json` when a CI job needs a report rather than human text.

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

## `tysel run`

```text
tysel run [ENTRY] [--manifest FILE]
```

Bundle and serve without file watching. An explicit `ENTRY` overrides
`app.entry` for this run. Runtime-relative paths resolve from the selected
project root.

For HTTP and process behavior, see [Application and server](../manifest/app-server.md)
and [Errors and machine output](../errors-and-output.md).
