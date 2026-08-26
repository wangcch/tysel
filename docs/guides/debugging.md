# Debug service failures safely

This guide follows one request from the HTTP error envelope to development
source information and capability logs, then replaces unsafe runtime details
with an application-owned production response.

## Reproduce in development

Run the service with the same manifest limits and permissions used by the
failing deployment:

```sh
tysel check
tysel inspect
tysel dev
```

`tysel dev` bundles TypeScript with a generated source map and uses it to
symbolicate handler failures. Trigger the smallest request that reproduces the
problem and capture the status, response error code, fixed-width hexadecimal
`requestId`, and matching JSON log `rid`.

An unhandled throw or rejected handler becomes:

```json
{
  "error": {
    "code": "RUNTIME_ERROR",
    "message": "...",
    "requestId": "000000000000002a"
  }
}
```

The response ID is hexadecimal while the structured log `rid` is numeric.
They identify the same process-local request when converted between bases.
They reset after process restart and are not distributed trace IDs.

## Classify the boundary first

| HTTP status and code | Boundary | First action |
| --- | --- | --- |
| `413 BODY_TOO_LARGE` | Request exceeds `max_request_mb` | Reject or chunk at the caller; raise only for a measured payload need. |
| `500 RESPONSE_TOO_LARGE` | Buffered response exceeds `max_response_mb` | Paginate, stream within the bound, or reduce output. |
| `503 OVERLOADED` | `max_in_flight` admission has no permit | Back off idempotent work and inspect service time and capacity. |
| `500 RUNTIME_ERROR` | Handler, runtime, timeout, or capability failure | Correlate the request with logs and reproduce under the same authority. |
| `500 INTERNAL_ERROR` with minimal body | Error-envelope serialization fallback | Preserve host diagnostics and treat as a runtime fault. |

A returned `Response` is controlled by the application. Only uncaught failures
use the runtime envelope. Capability logs identify a bounded capability,
operation, and `ok`, `denied`, or `error` result without exposing SQL, paths,
URLs, or secrets.

## Understand source-map scope

Source maps are generated during bundling and embedded in the application
package. Today, the development server parses and applies them to HTTP
failures. The packaged service runtime does not currently attach the embedded
map to its HTTP dispatcher, so a production `RUNTIME_ERROR` may contain
generated bundle locations rather than original TypeScript locations.

Keep the exact source revision, lockfile, manifest, executable digest, and
release sidecars for every deployment. Reproduce the same request with that
revision under `tysel dev`; do not depend on returning production stack text
to clients as the debugging channel.

For Component tasks there is no HTTP envelope. A success writes one JSON value
to stdout; ABI, policy, guest, resource, and output-validation failures go to
stderr and exit non-zero. The guest WIT error text is a bounded human
diagnostic, not a stable code.

## Map expected errors in the application

Catch failures at the narrow boundary where the application can choose a safe
status and message. In this example, `route` is the application's existing
router and the logged category is deliberately allowlisted:

```ts
function safeCategory(error: unknown): "type" | "unexpected" {
  return error instanceof TypeError ? "type" : "unexpected";
}

export default {
  async fetch(request: Request): Promise<Response> {
    try {
      return await route(request);
    } catch (error) {
      console.error("request failed", safeCategory(error));
      return Response.json(
        { error: { code: "DEPENDENCY_UNAVAILABLE", message: "Try again later" } },
        { status: 503 },
      );
    }
  },
};
```

Keep `safeCategory` bounded; do not log request bodies, authorization headers,
secret values, connection URLs, SQL, or raw provider responses. Map validation,
not-found, conflict, denial, and dependency failures to deliberate application
codes. Leave truly unexpected faults uncaught during development so they remain
visible.

## Compare project and packaged behavior

Before closing an incident, test both execution paths:

```sh
tysel test --json
tysel build --release --output dist/app
./dist/app
```

Project commands resolve relative capability roots from the manifest
directory. The packaged executable resolves them from its process working
directory. `.env` participates in local development but is not packaged.
Differences in working directory, injected secrets, Postgres URLs, OTLP
configuration, proxy behavior, or operating-system permissions often explain
“works in dev” failures.

For CLI automation, use `--error-format json`. Fatal setup errors are one
`TYSEL_CLI_ERROR` object on stderr and exit `1`; command reports go to stdout
and may coexist with a non-zero policy result. Parse the streams separately.

See [Errors and machine output](../reference/errors-and-output.md),
[Concurrency and backpressure](concurrency-backpressure.md), and
[Production incident response](../operations/production.md#incident-response).
