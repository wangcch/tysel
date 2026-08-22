# Hello service

The smallest Tysel HTTP application: one Fetch handler, no host capabilities,
and one executable output.

From this example directory:

```sh
tysel config validate
tysel check
tysel run
```

Call the listener:

```sh
curl http://127.0.0.1:3000/hello
```

The response contains the greeting and request path. Package it from the same
directory:

```sh
tysel build --release
./dist/hello-service
```

Use `tysel init my-service` to generate the same structure in a new directory.
`--manifest-format json` generates an equivalent `tysel.json`.
