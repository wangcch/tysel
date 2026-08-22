# Hono API

This example runs a small [Hono](https://hono.dev/) router on Tysel's
Web-standard request and response APIs.

From this example directory, install workspace dependencies, then validate and
start it:

```sh
pnpm install
tysel config validate
tysel check
tysel run
```

```sh
curl http://127.0.0.1:3000/
curl http://127.0.0.1:3000/hello/Tysel
```

This example demonstrates a compatible, Web-API-first npm dependency. It does
not imply general npm or Node.js compatibility; run `tysel compat` and tests
for each dependency used by an application.
