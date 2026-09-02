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

## Container image

`tysel.container.toml` is the complete deployment manifest. It differs from
`tysel.toml` only by binding the listener to `0.0.0.0:3000`.

Build the application and runtime image from this directory with an exact
published Tysel version that provides the matching toolchain image:

```sh
docker build \
  --build-arg TYSEL_VERSION=VERSION \
  --tag hello-service:local .
docker run --rm --publish 127.0.0.1:3000:3000 hello-service:local
```

The build stage pulls
`ghcr.io/wangcch/tysel-toolchain:VERSION`; the final application image contains
only the generated executable. Pin the toolchain image by digest in production.

The Fetch handler returns HTTP 200 for `/healthz`, so it can be used as the
example's external container probe:

```sh
curl --fail http://127.0.0.1:3000/healthz
```

To package an already admitted Linux executable instead, build it with the
container manifest and use `Dockerfile.runtime`:

```sh
tysel build --release \
  --manifest tysel.container.toml \
  --output dist/hello-service
docker build \
  --file Dockerfile.runtime \
  --tag hello-service:local .
```
