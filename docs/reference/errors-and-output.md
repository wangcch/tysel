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

Fatal errors are one JSON object on stderr:

```json
{
  "error": {
    "code": "TYSEL_CLI_ERROR",
    "message": "manifest validation failed: ..."
  }
}
```

`code` is currently the generic `TYSEL_CLI_ERROR`; use `message` for a human
diagnostic, not as a stable programmatic discriminator. The process exits with
status `1` for this fatal path.

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

Runtime messages can contain symbolicated application stack information in
development. Treat them as operator diagnostics: map errors to an
application-owned response before exposing sensitive internals to public
clients.

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

See [Application module](runtime/application.md), [Application limits](manifest/limits.md),
and [Production operations](../operations/production.md).
