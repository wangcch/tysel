# Host capabilities

HTTP handlers receive host methods through their `runtime` argument. The same
object remains available as `globalThis.tysel` for compatibility and for task
or durable handlers. Type availability does not grant authority: the execution
profile, manifest permission, and host configuration must all permit the call.

## Capability summary

| Member | Main operations | Required authority |
| --- | --- | --- |
| `tysel.secrets` | `ref(name)` | Declared `permissions.secrets` name. |
| `tysel.sqlite` | `exec(sql, params?)`, `query(sql, params?)` | Trusted profile. |
| `tysel.postgres` | `exec(sql, params?)`, `query(sql, params?)` | Named `permissions.postgres` grant and connection URL. |
| `tysel.redis` | `get`, `set`, `del`, `exists`, `expire` | Named `permissions.redis` grant and connection URL. |
| `tysel.fs` | `read(path)`, `write(path, contents)` | Matching `fs_read` or `fs_write` root. |
| `tysel.llm` | `generate(options)` | Provider configuration and declared provider secret. |
| `tysel.acceptWebSocket()` | Accept current inbound upgrade | HTTP/1.1 WebSocket enabled and trusted profile. |
| `new WebSocket(url)` | Outbound connection | Trusted profile and matching `permissions.fetch` host. |

## Runtime utilities

```ts
interface TyselRuntime {
  readonly isolateId: number;
  sleep(milliseconds: number): Promise<void>;
  echo(value: string): Promise<string>;
  httpGet(url: string): Promise<Response>;
  // capability clients are listed below
}
```

`isolateId` identifies the current native isolate. `sleep` is a transient timer,
not a durable suspension. Prefer standard `fetch` for application HTTP;
`httpGet` is a narrow host helper. `echo` is primarily useful for validating
the host bridge.

For the profile-by-profile matrix, see [Capabilities](../../capabilities/README.md).

## Secrets

```ts
const token = await tysel.secrets.ref("API_TOKEN");
```

`ref` returns an opaque branded `SecretReference` such as
`secret:API_TOKEN`, never the plaintext environment value. Pass the reference
only to host APIs that explicitly accept it.

## SQL

Both SQL clients accept positional parameter arrays. `exec` resolves to the
affected-row count; `query` resolves to an array of row objects.

```ts
await tysel.sqlite.exec(
  "create table if not exists jobs (id text primary key, state text)",
);
await tysel.sqlite.exec(
  "insert into jobs (id, state) values (?, ?)",
  ["job_1", "ready"],
);
const jobs = await tysel.sqlite.query("select id, state from jobs");

const remote = await tysel.postgres.query(
  "select id, state from jobs where id = $1",
  ["job_1"],
);
```

A read-only Postgres grant rejects writes. SQL text, parameter count, rows, and
result bytes are bounded; see [Limits and defaults](../limits-and-defaults.md).
Use the [SQLite](../../guides/sqlite.md) or
[PostgreSQL](../../guides/postgresql.md) guide for an end-to-end deployment path.

## Redis

```ts
await tysel.redis.set("session:123", "active", { ttlSeconds: 300 });
const state = await tysel.redis.get("session:123");
```

Values are UTF-8 strings. Keys, values, TTLs, deletion batches, and concurrent
connections are bounded. See the [Redis guide](../../guides/redis.md).

## Filesystem

```ts
const source = await tysel.fs.read("./fixtures/input.json");
await tysel.fs.write("./data/output/result.json", source);
```

Operations use UTF-8 strings and regular files beneath pinned roots. Paths do
not escape a declared root through traversal or symlink substitution.
Use the [filesystem guide](../../guides/filesystem.md) to prepare roots and
verify allowed and denied paths.

## LLM generation

```ts
const response = await tysel.llm.generate({
  model: "default",
  system: "Answer with one sentence.",
  input: "Summarize the queued job.",
  maxOutputTokens: 128,
  temperature: 0.2,
});

console.log(response.output, response.usage, response.provider_request_id);
```

`model` and `input` are required. `system`, `maxOutputTokens`, and
`temperature` are optional. The response contains `output`, required token
usage metadata, and an optional provider request identifier. Provider endpoint,
model aliases, and secret selection come from [environment variables](../environment.md).

## WebSockets

Call `tysel.acceptWebSocket()` only while handling an eligible inbound upgrade.
For outbound connections, `WebSocket.opened` is the connection promise. The
current runtime supports text and binary frames but not browser subprotocols,
extensions, `Blob`, or `bufferedAmount`; one outbound socket may be active per
isolate request.

An accepted inbound socket has `readyState`, `onmessage`, `onclose`, `onerror`,
`send(string)`, `close()`, and `addEventListener` / `removeEventListener` for
`message`, `close`, and `error`. `send` and `close` return promises. The
accepted-socket type is separate from the outbound Web `WebSocket` global.

Use the [WebSocket contract](https://tysel.dev/reference/javascript/websocket/)
and [fetch contract](https://tysel.dev/reference/javascript/fetch/) for the
exact supported subsets.
