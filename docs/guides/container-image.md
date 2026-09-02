# Container image reference

Use one of these image layouts:

| Layout | Build location | Use when |
| --- | --- | --- |
| [Multi-stage Dockerfile](#build-from-source-in-docker) | Docker build stage | You want `docker build` to compile the application. |
| [Runtime-only Dockerfile](#package-an-existing-executable) | Linux build job before Docker | You promote a verified executable and its release evidence. |
| [`tysel image`](#generate-the-runtime-image) | Linux host, unless `--binary` is used | You want Tysel to generate the runtime-only context. |

The final image contains one Tysel executable. It does not need Tysel, Node.js,
npm, or `node_modules` at runtime.

## Container manifest

Tysel does not merge manifests. Copy the complete development manifest, then
change only the listener when the remaining production settings are identical:

```sh
cp tysel.toml tysel.container.toml
```

In `tysel.container.toml`:

```toml
[server]
listen = "0.0.0.0:3000"
```

Do not replace the rest of the manifest with this fragment. Permissions,
limits, durable settings, observability, and project tasks must remain in the
complete file. The manifest used by `tysel build` is embedded in the
executable. See the checked-in
[development manifest](https://github.com/wangcch/tysel/blob/main/examples/hello-service/tysel.toml)
and
[container manifest](https://github.com/wangcch/tysel/blob/main/examples/hello-service/tysel.container.toml).

## Build from source in Docker

This checked-in
[Dockerfile](https://github.com/wangcch/tysel/blob/main/examples/hello-service/Dockerfile)
uses the matching Tysel toolchain image to compile the application, then copies
only the executable into the runtime stage:

```dockerfile
# syntax=docker/dockerfile:1
ARG TYSEL_VERSION
ARG TYSEL_TOOLCHAIN_IMAGE=ghcr.io/wangcch/tysel-toolchain:${TYSEL_VERSION}
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot

FROM ${TYSEL_TOOLCHAIN_IMAGE} AS build
WORKDIR /src
COPY . .

RUN tysel task verify \
    --manifest tysel.container.toml
RUN tysel build --release \
    --manifest tysel.container.toml \
    --output /out/tysel-app

FROM ${RUNTIME_IMAGE}
# CI may add version, source, revision, and admitted artifact digest labels with
# docker build --label. Do not substitute the registry image digest for the
# executable digest from the verified .sha256 sidecar or `tysel image` output.
WORKDIR /app
COPY --from=build --chown=65532:65532 /out/tysel-app /app/tysel-app
USER 65532:65532
EXPOSE 3000
ENTRYPOINT ["/app/tysel-app"]
```

Build it from the project root:

```sh
docker build \
  --build-arg TYSEL_VERSION=VERSION \
  --tag hello-service:local .
```

Replace `VERSION` with an exact published Tysel version. The checked-in hello
service has no package dependencies. The toolchain image contains
`tysel`, `tysel-service`, and `tysel-worker`; it does not contain Node.js or a
JavaScript package manager.

If the application has a `package-lock.json`, install its locked dependencies
in a separate Node build stage and copy only `node_modules` into the Tysel
build stage:

```dockerfile
ARG TYSEL_VERSION
FROM node:22-bookworm-slim AS dependencies
WORKDIR /src
COPY package.json package-lock.json ./
RUN npm ci

FROM ghcr.io/wangcch/tysel-toolchain:${TYSEL_VERSION} AS build
WORKDIR /src
COPY --from=dependencies /src/node_modules ./node_modules
COPY . .
RUN tysel task verify --manifest tysel.container.toml
RUN tysel build --release \
    --manifest tysel.container.toml \
    --output /out/tysel-app
```

Use the Node version and locked install command required by the project. Do not
copy host `node_modules` across operating systems.

The release workflow publishes `ghcr.io/wangcch/tysel-toolchain:VERSION` as a
Linux amd64/arm64 OCI index for releases that include toolchain-image support.
It is assembled from the same signed Linux release archives, not from a second
Cargo build. Its Debian base image is pinned by multi-platform index digest.
The registry digest is the immutable image identity; resolve the version tag
after release and pin that digest in production Dockerfiles.
The OCI index itself is not covered by Tysel's release-manifest signature;
image signing and registry admission remain separate deployment policy.

GHCR creates a new package as private. On the first toolchain-image release,
the workflow intentionally stops if the published package is not anonymously
readable. A maintainer must link it to the repository if necessary, change its
visibility to Public, and rerun the failed release jobs. Later releases retain
that package visibility.

This is a build image, not an application runtime image. It runs as root so a
Docker build can create outputs in its workspace. Do not deploy it as the HTTP
service. The final stage above remains non-root and contains only the generated
application executable.

The example's checked-in `.dockerignore` is:

```text
.git
dist
node_modules
*.log
.env*
```

The multi-stage layout does not export the checksum, compatibility, SBOM,
license, and evidence sidecars outside the build container. Use the
runtime-only layout when those files are release admission inputs.

## Package an existing executable

Build the complete artifact set in a Linux release job:

```sh
tysel task verify --manifest tysel.container.toml
tysel build --release \
  --manifest tysel.container.toml \
  --output dist/hello-service
```

The hello-service example checks in this
[runtime-only Dockerfile](https://github.com/wangcch/tysel/blob/main/examples/hello-service/Dockerfile.runtime):

```dockerfile
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot
FROM ${RUNTIME_IMAGE}
WORKDIR /app
COPY --chown=65532:65532 dist/hello-service /app/tysel-app
USER 65532:65532
EXPOSE 3000
ENTRYPOINT ["/app/tysel-app"]
```

Then build the image:

```sh
docker build \
  --file Dockerfile.runtime \
  --tag registry.example/hello-service:VERSION .
```

Store `dist/hello-service` and its five release sidecars together outside the
image. The runtime image needs only the executable.

## Generate the runtime image

`tysel image` generates the same runtime-only Dockerfile.

| Invocation | Application build | Docker build |
| --- | --- | --- |
| `tysel image` | Yes, in release mode on Linux | Yes |
| `tysel image --context-only` | Yes, in release mode on Linux | No |
| `tysel image --binary PATH` | No; copies `PATH` | Yes |
| `tysel image --binary PATH --context-only` | No; copies `PATH` | No |

Build the application and image in one command:

```sh
tysel image \
  --manifest tysel.container.toml \
  --tag registry.example/hello-service:VERSION
```

Generate a context for inspection or a separate Docker build:

```sh
tysel image \
  --manifest tysel.container.toml \
  --context-only \
  --output-dir dist/image

docker build --tag registry.example/hello-service:VERSION dist/image
```

Package an admitted executable without rebuilding it:

```sh
tysel image \
  --manifest tysel.container.toml \
  --binary dist/hello-service \
  --copy-sidecars \
  --tag registry.example/hello-service:VERSION
```

`--binary` requires a 64-bit little-endian Linux x86-64 or arm64 Tysel
executable with embedded TAP metadata. The embedded application name, profile,
and listener must match the selected manifest. Build `dist/hello-service` with
`tysel.container.toml`; passing that manifest only to `tysel image` is not
sufficient.

`--copy-sidecars` first verifies the checksum, compatibility, SBOM, license,
and evidence files, checks that the evidence target matches the executable's
ELF architecture, then copies those five files into the generated context.
The Dockerfile still copies only `tysel-app` into the runtime image. This option
is for a CI job that needs the admitted evidence beside the Docker build
context; it does not regenerate missing evidence.

Select another Docker-compatible builder with either form:

```sh
tysel image --builder podman --manifest tysel.container.toml
DOCKER=/usr/local/bin/podman tysel image --manifest tysel.container.toml
```

`--builder` takes precedence over `DOCKER`; both name one executable and cannot
contain embedded arguments.

The generated Dockerfile records the executable digest, execution profile,
runtime version, and application title as labels. Add release metadata when it
is available:

```sh
tysel image \
  --manifest tysel.container.toml \
  --image-version VERSION \
  --label org.opencontainers.image.source=https://github.com/OWNER/REPOSITORY
```

Do not put credentials or other secrets in labels. The executable digest label
does not replace the registry digest used for deployment.
Label values are preserved literally, including `$` characters.

When `--base-image` is left at its default, `tysel image` accepts glibc-linked
and static executables and rejects a musl interpreter. With a custom base,
Tysel still validates the ELF format, architecture, and interpreter structure,
but cannot infer which libc or shared libraries exist in that image. That
compatibility becomes the image author's responsibility.

## Run and probe

```sh
docker run --rm --publish 127.0.0.1:3000:3000 hello-service:local
curl --fail http://127.0.0.1:3000/healthz
```

The distroless runtime has no shell or `curl`, so use an external HTTP probe
instead of an exec probe. Tysel does not add a health route.

## Runtime contract

| Requirement | Setting |
| --- | --- |
| User | Keep numeric user/group `65532:65532`. |
| Root filesystem | Use read-only; add only the writable mounts below. |
| Temporary files | Mount a bounded `tmpfs`, for example at `/tmp`. |
| SQLite or relative filesystem paths | Mount a writable volume at the path resolved from `/app`. |
| Secrets and database URLs | Inject environment values at runtime, never with Dockerfile `ARG` or `ENV`. |
| Health check | Probe an application route over HTTP from outside the container. |
| Production identity | Push a version tag, record the registry digest, and deploy by digest. |
| Rollback | Redeploy the previously admitted digest; do not rebuild an old tag. |

Do not put secrets in the Dockerfile, build arguments, manifest, image labels,
or source tree.

## Errors

| Error | Fix |
| --- | --- |
| Service must listen on `0.0.0.0` or `[::]` | Build with a container manifest whose `[server].listen` uses an unspecified address and non-zero port. |
| Linux executable required | Run the build on Linux, or pass a matching Linux ELF with `--binary`. |
| Executable has no TAP metadata or differs from the selected manifest | Rebuild it with the same complete container manifest passed to `tysel image`. |
| Component profile is rejected | Package the one-shot executable using [Component tasks](../operations/component-tasks.md). |
| Release sidecar verification fails | Rebuild the artifact with `tysel build --release`; do not copy an incomplete or modified evidence set. |
| Generated files already exist | Choose another output directory or review and pass `--force`. |
| Container builder cannot start | Set `--builder` or `DOCKER`, or add `--context-only` and build the context elsewhere. |
| Container starts but is unreachable | Compare the embedded listener, `EXPOSE`, published port, and proxy target. |
| Executable fails in the runtime image | Confirm its ELF interpreter and native libraries are compatible with the selected base image. |

See [Build and image commands](../reference/cli/delivery.md),
[Reproducible release](reproducible-release.md), and
[Production operations](../operations/production.md).
