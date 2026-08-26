# Persist application data with SQLite

This guide configures the runtime-owned SQLite client, executes parameterized
statements, verifies persistence, and separates the application database from
the durable event log.

## Choose a stable path

SQLite is available to the trusted `service` profile when the durable store is
`sqlite`, which is the manifest default. State the path explicitly for a
deployable application:

```toml
[app]
name = "sqlite-worker"
entry = "src/index.ts"
profile = "service"

[durable]
store = "sqlite"
path = "./data/application.db"
```

Project commands resolve a relative path from the manifest directory. A
packaged executable resolves it from the process working directory. The runtime
creates missing parent directories, but production should mount a stable,
writable volume and set a deterministic working directory.

## Execute parameterized SQL

```ts
await tysel.sqlite.exec(
  "CREATE TABLE IF NOT EXISTS counters (key TEXT PRIMARY KEY, value INTEGER NOT NULL)",
);
await tysel.sqlite.exec(
  "INSERT INTO counters(key, value) VALUES (?, 1) " +
    "ON CONFLICT(key) DO UPDATE SET value = value + 1",
  ["visits"],
);
const rows = await tysel.sqlite.query(
  "SELECT key, value FROM counters WHERE key = ?",
  ["visits"],
);
```

`exec` resolves to the affected-row count. `query` resolves to row objects.
Parameters must be an array of `null`, boolean, finite number, or string
values. Never interpolate request values into SQL text.

Run the complete [SQLite worker](https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker):

```sh
cd examples/sqlite-worker
tysel config validate
tysel check
tysel run
```

From another terminal:

```sh
curl 'http://127.0.0.1:3000/?key=visits'
curl 'http://127.0.0.1:3000/?key=visits'
```

The second response reports `value: 2`. Stop and restart the process, call it
again, and confirm the value persists in the configured database.

## Runtime behavior and bounds

The current runtime owns one process-wide SQLite connection, serializes access
with a mutex, and applies a 5-second SQLite busy timeout. Multiple JavaScript
workers do not create independent connection pools. Long statements therefore
delay other SQLite operations in the same process.

Current fixed bounds are:

| Surface | Bound |
| --- | ---: |
| SQL text | 1 MiB |
| Parameters | 999 |
| Returned rows | 10,000 |

When an SQL operation fails, Tysel attempts to roll back an open transaction so
the shared connection remains usable. Prefer short transactions and avoid
holding a transaction across unrelated HTTP requests.

SQLite is denied in the `isolated` profile. It is also distinct from Component
filesystem imports; a Component cannot obtain `tysel.sqlite` by requesting a
filesystem root.

## Separate application and durable data

`durable.path` selects the application-facing SQLite capability. When the
module also exports durable handlers, Tysel places `durable-events.db` beside
that application database unless `TYSEL_DURABLE_SQLITE_PATH` or
`TYSEL_DURABLE_POSTGRES_URL` selects another event store.

Back up both files when both features are active. Restore them as one
application-consistent set, and follow the
[production storage guidance](../operations/production.md#durable-postgres-backup-and-restore)
before resuming schedulers.

## Production checklist

- use persistent local storage with adequate free space and inode capacity;
- keep one application writer per database unless the deployment has been
  tested for its exact locking pattern;
- use parameterized statements and bound response size;
- monitor query latency, busy failures, disk latency, and filesystem capacity;
- back up and restore-test the application database;
- use Postgres when the workload requires remote multi-host coordination,
  larger concurrency, or database-managed availability.

See the [SQLite API](../reference/runtime/capabilities.md#sql),
[durable configuration](../reference/manifest/durable-observability.md), and
[data capability limits](../reference/limits-and-defaults.md#data-capabilities).
