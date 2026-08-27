# Redis

Tysel exposes a deliberately small Redis API for cache entries, idempotency
keys, and short-lived coordination state. Arbitrary Redis commands, Lua scripts,
Pub/Sub, streams, and cluster routing are not part of version 1.

## Declare a named grant

```toml
[app]
name = "redis-service"
entry = "src/index.ts"
profile = "service"

[permissions]
redis = ["cache:read-write"]
```

The deployment supplies the URL:

```sh
export TYSEL_REDIS_CACHE='rediss://app:password@redis.example.com:6380/0'
```

Use `redis://` for local development and certificate-validated `rediss://` in
production. Never put the URL or credentials in the manifest or build artifact.

## Use the bounded API

```ts
const previous = await tysel.redis.get("job:123");
await tysel.redis.set("job:123", "running", { ttlSeconds: 300 });
const present = await tysel.redis.exists("job:123");
const extended = await tysel.redis.expire("job:123", 600);
const removed = await tysel.redis.del("job:123", "job:124");
```

`get` returns a UTF-8 string or `null`. `set` accepts an optional TTL. `exists`
and `expire` report whether the key exists; `del` returns the number of removed
keys. A `read-only` grant permits only `get` and `exists`.

| Surface | Bound |
| --- | ---: |
| Key | 4 KiB |
| Value | 1 MiB |
| Keys per `del` | 128 |
| TTL | 1 second–365 days |
| Concurrent operations | 4 |

The runtime reuses one multiplexed connection and reconnects automatically
after connection loss. The API is available only in the `service` profile. Use Redis ACLs in addition
to the manifest mode, set server-side memory and eviction policies explicitly,
and treat Redis as disposable unless the deployment has a tested persistence
and recovery plan.
