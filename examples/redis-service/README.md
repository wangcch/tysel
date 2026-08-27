# Redis service

This service demonstrates a named, read-write Redis grant. The manifest
contains only the grant name and mode; the connection URL stays in the host
environment.

```sh
export TYSEL_REDIS_CACHE='redis://127.0.0.1:6379/0'
tysel config validate
tysel check
tysel run
```

Call `http://127.0.0.1:3000/?key=greeting`. The first request writes a value
with a 60-second TTL and later requests read it. For query-only applications,
change the grant to `cache:read-only`.
