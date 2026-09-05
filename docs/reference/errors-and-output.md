# Errors and machine output

Tysel separates human diagnostics from stable JSON envelopes. Automation
should select a machine-readable mode explicitly and parse stdout and stderr
independently.

## CLI exit status

| Status | Meaning |
| --- | --- |
| `0` | Command completed successfully. |
| Non-zero | Validation, compatibility policy, test, build, benchmark gate, runtime, or command usage failed. |

Do not infer a specific failure kind from the numeric status alone. Use the
selected JSON report or fatal error envelope.

## Fatal CLI errors

The option is global and can appear before or after the command:

```sh
tysel --error-format json check
```

Fatal errors are one JSON object on stderr. Build failures also include a
`diagnostics` array with original-source positions when Oxc supplies a span:

```json
{
  "error": {
    "code": "TYSEL_CLI_ERROR",
    "message": "manifest validation failed: ..."
  },
  "diagnostics": [{
    "code": "TYSEL_PARSE_ERROR",
    "message": "...",
    "severity": "error",
    "phase": "parse",
    "file": "src/index.ts",
    "start": { "line": 4, "column": 7, "byteOffset": 52 },
    "end": { "line": 4, "column": 8, "byteOffset": 53 }
  }]
}
```

`code` is currently the generic `TYSEL_CLI_ERROR`; use `message` for a human
diagnostic, not as a stable programmatic discriminator. The process exits with
status `1` for this fatal path.

Individual static diagnostics use these stable codes:

| Code | Meaning |
| --- | --- |
| `TYSEL_MANIFEST_PARSE_ERROR` | TOML/JSON syntax or deserialization failed. |
| `TYSEL_MANIFEST_INVALID` | A manifest semantic rule failed. |
| `TYSEL_NODE_BUILTIN_UNSUPPORTED` | A runtime import requests a Node builtin. |
| `TYSEL_IMPORT_UNRESOLVED` | A runtime import cannot be resolved. |

Manifest diagnostics use phase `manifest`; import diagnostics use `resolve`.
Manifest validation reports the first failure, with its field range when
available. Missing fields may have no range, and parser errors may point to an
insertion point or enclosing table. Import diagnostics cover modules reached by
the build resolver, including dependencies, and refer to original TypeScript
source rather than emitted JavaScript. Erased type-only imports are excluded.
TypeScript typecheck output itself remains compiler text, not this structured
protocol.


## Development diagnostic stream

`tysel --error-format json dev` writes one JSON object per diagnostic update to
stderr while the server continues running. A failed reload publishes a
non-empty array; the next successful reload publishes an empty array so an
editor can clear stale diagnostics. `generation` increases for each reload
attempt, and `schemaVersion` is currently `1`.

Unhandled handler failures publish a separate `runtimeDiagnostic` event
containing one source-mapped diagnostic and hexadecimal `requestId`. It is an
occurrence, not a replacement for the current build-diagnostic snapshot.
Runtime stacks are bounded to 64 KiB and pass through a bounded 64-entry
queue. If that queue overflows, Tysel emits `TYSEL_DIAGNOSTICS_DROPPED` after
output resumes. Lines and columns are one-based; `byteOffset` is a zero-based
UTF-8 offset when supplied by the compiler.

## Command reports

Machine-readable command reports go to stdout:

```sh
tysel test --json
tysel compat --json
tysel bench all --format json
tysel doctor --json
tysel upgrade --check --json
```

A JSON report and a non-zero status can coexist when the command completed its
analysis but found a failing policy or test. Always check both. Keep stderr
available for fatal setup failures that prevent report generation.

## HTTP error envelope

Unhandled runtime failures return HTTP `500` with JSON:

```json
{
  "error": {
    "code": "RUNTIME_ERROR",
    "message": "...",
    "requestId": "000000000000002a"
  }
}
```

An oversized inbound body returns HTTP `413` with the same shape and code
`BODY_TOO_LARGE`. `requestId` is a fixed-width hexadecimal correlation value.
An internal serialization fallback can return only
`{"error":{"code":"INTERNAL_ERROR"}}`.

`tysel dev` uses the generated source map to symbolicate application stack
information. Although the map is embedded during packaging, the packaged HTTP
dispatcher does not currently apply it, so production messages can contain
bundle locations. Treat all runtime messages as operator diagnostics: map
errors to an application-owned response before exposing sensitive internals to
public clients.

## Application errors

Tysel does not impose an error schema on responses returned by your fetch
handler. Define application status codes and JSON envelopes deliberately, and
include a safe correlation identifier. A returned `Response` is an application
response; a thrown or rejected handler becomes the runtime `500` envelope.

## Component process output

A successful Component invocation writes exactly one JSON value followed by a
newline to stdout and exits `0`. Compilation, ABI, policy, guest error,
resource-limit, or output-validation failures are fatal CLI/runtime errors on
stderr and exit non-zero; they do not use the HTTP error envelope.

The Component guest's WIT error branch is retained as a UTF-8 diagnostic of at
most 4 KiB. Treat its text as human-readable rather than a stable error code.
See [Component ABI](component/abi.md) and
[Component runtime limits](component/runtime.md#resource-bounds).

See [Debug service failures](../guides/debugging.md),
[Application module](runtime/application.md), [Application limits](manifest/limits.md),
and [Production operations](../operations/production.md).
