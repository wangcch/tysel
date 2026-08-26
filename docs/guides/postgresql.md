# Connect to PostgreSQL

This guide grants one named PostgreSQL connection, injects its URL from the
host, uses parameterized SQL, verifies read-only enforcement, and prepares the
connection for production deployment.

## Declare a name, not a URL

```toml
[app]
name = "postgres-service"
entry = "src/index.ts"
profile = "service"

[permissions]
postgres = ["main:read-write"]
```

A grant is `name:read-write` or `name:read-only`; omitting the mode means
`read-write`. Version 1 accepts at most one PostgreSQL grant. Names begin with a
lowercase letter and then use lowercase letters, digits, `_`, or `-`.

The URL belongs to the deployment:

```sh
export TYSEL_POSTGRES_MAIN='postgres://app:password@db.example.com:5432/app?sslmode=require'
```

Uppercase the grant name and replace `-` with `_`: `review-ro` maps to
`TYSEL_POSTGRES_REVIEW_RO`. `tysel dev` may read this selected value from
`.env`, but packaged deployments should inject it through a service manager or
secret manager. Never put a URL in the manifest, CLI arguments, build artifact,
or logs.

## Execute parameterized SQL

```ts
await tysel.postgres.exec(
  "INSERT INTO jobs (id, state) VALUES ($1, $2) " +
    "ON CONFLICT (id) DO UPDATE SET state = EXCLUDED.state",
  ["job_123", "ready"],
);
const rows = await tysel.postgres.query(
  "SELECT id, state FROM jobs WHERE id = $1",
  ["job_123"],
);
```

`exec` returns the affected-row count; `query` returns row objects. Parameters
accept `null`, boolean, finite number, and string values. Arrays and objects are
not directly bindable. Cast or serialize intentionally when a schema uses
JSON, UUID, timestamps, or another database-specific type.

Run the complete [Postgres service](https://github.com/wangcch/tysel/tree/main/examples/postgres-service):

```sh
cd examples/postgres-service
export TYSEL_POSTGRES_MAIN='postgres://app:password@127.0.0.1:5432/app'
tysel config validate
tysel check
tysel run
```

Call `http://127.0.0.1:3000/`. The response contains the inserted greeting row.

## Enforce read-only access

For a query-only application:

```toml
[permissions]
postgres = ["main:read-only"]
```

The host rejects every `tysel.postgres.exec` call before sending SQL. Each
`query` runs inside `BEGIN READ ONLY`; attempting write SQL through `query` also
fails. PostgreSQL read-only violations use the safe message
`postgres connection is read-only (SQLSTATE 25006)`.

Verify the boundary by switching the example grant to read-only and calling the
handler. Its table creation or upsert must fail. Restore read-write only for the
deployment role that actually owns schema or data mutation.

Read-only enforcement is an application boundary, not a replacement for
database roles. Give the URL a database user whose PostgreSQL privileges match
the manifest mode.

## Pooling, types, and bounds

The runtime maintains up to four PostgreSQL connections for the configured
grant and reuses healthy idle sessions. A checkout waits for a slot within the
application invocation deadline. Connection setup uses certificate-validating
native TLS when the URL requests TLS.

| Surface | Bound |
| --- | ---: |
| SQL text | 1 MiB |
| Parameters | 999 |
| Returned rows | 10,000 |
| Serialized result | 1 MiB |
| Runtime connections | 4 |

Returned columns currently support null, boolean, integer, floating-point,
text-compatible, and byte values. Cast unsupported database-specific types to
a supported representation in SQL. Database errors expose a bounded SQLSTATE
rather than the server's full error detail; connection and TLS failures remain
operational diagnostics and should not be returned directly to clients.

## Production checklist

- require certificate-validated TLS and verify the deployment URL;
- store URLs in a secret manager and prevent them from entering logs;
- align the manifest mode with a least-privilege PostgreSQL role;
- apply schema migrations as a separate controlled deployment step;
- size the database and any external proxy for four connections per process;
- set statement and ingress deadlines below caller timeouts;
- monitor checkout latency, SQLSTATE classes, saturation, rows, and result size;
- test database failover and connection recreation with the exact driver path.

Application PostgreSQL is separate from the durable event-store variable
`TYSEL_DURABLE_POSTGRES_URL`. Configure and recover them according to their own
data ownership even when they point at the same server.

See [Postgres permissions](../reference/manifest/permissions.md#postgres),
[Environment variables](../reference/environment.md#application-capabilities),
and [data capability limits](../reference/limits-and-defaults.md#data-capabilities).
