# Permissions

The optional `[permissions]` table grants host capabilities. Every list
defaults to empty, so access is denied unless declared.

| Field | Value form | Current schema bound | Enables |
| --- | --- | --- | --- |
| `fetch` | Unique hostnames | — | Outbound `fetch` and WebSocket connections to matching hosts. |
| `secrets` | Unique environment-variable names | — | Opaque secret references through `tysel.secrets`. |
| `postgres` | One unique named grant | 1 item | A named Postgres connection and access mode. |
| `fs_read` | Unique directory roots | 64 items | UTF-8 regular-file reads beneath pinned roots. |
| `fs_write` | Unique directory roots | 64 items | UTF-8 regular-file writes beneath pinned roots. |

Empty entries and duplicates are rejected. Grant presence does not override an
execution profile that disallows the capability.

```toml
[permissions]
fetch = ["api.example.com"]
secrets = ["API_TOKEN"]
postgres = ["main:read-only"]
fs_read = ["./fixtures"]
fs_write = ["./data/output"]
```

## Outbound network

`fetch` entries are host allowlist values, not arbitrary URL prefixes. Scheme,
path, query, and credentials do not belong in the grant. Redirect targets are
checked independently against the allowlist.

## Secrets

Each item names an environment variable the host may expose as an opaque
reference. `tysel.secrets.ref("API_TOKEN")` returns a `secret:API_TOKEN`
handle; application JavaScript does not receive the plaintext environment
value. Declaring a name does not create or populate it.

## Postgres

A grant has the form `name:read-write` or `name:read-only`; an omitted mode
defaults to `read-write`. Names start with a lowercase letter and then contain
lowercase letters, digits, `_`, or `-`. At most one grant is currently
accepted.

Connection URLs do not belong in the manifest. For `main`, set
`TYSEL_POSTGRES_MAIN`; hyphens in a grant name map to underscores in the
environment variable.

Follow the [PostgreSQL guide](../../guides/postgresql.md) for parameter types,
read-only verification, pooling, TLS, and deployment checks.

## Filesystem

Roots resolve from the project root and are pinned before access. Reads and
writes are limited to UTF-8 regular files below the corresponding roots;
declaring `fs_read` does not imply write permission.

Follow the [filesystem guide](../../guides/filesystem.md) for root preparation,
path confinement, profile differences, denial checks, and a runnable transform.

See [Host capabilities](../runtime/capabilities.md) for method signatures,
[Environment variables](../environment.md) for host configuration, and the
[Security model](../../security/README.md) for enforcement boundaries.
