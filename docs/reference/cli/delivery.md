# Build and image commands

## `tysel build`

Bundle an application and emit one native executable.

```text
tysel build [ENTRY] [OPTIONS]
```

| Option | Meaning |
| --- | --- |
| `--target <triple>` | Validate the requested target. The current implementation only builds the host target. |
| `--profile <profile>` | Override the manifest execution profile. |
| `--release` | Build optimized output and emit release sidecars. |
| `--stub <path>` | Use an explicit runtime stub. |
| `-o, --output <path>` | Select the executable path. |
| `--manifest <file>` | Select one manifest. |

`ENTRY` overrides `app.entry`. A release build writes checksum,
compatibility, SBOM, license, and evidence sidecars next to the artifact.

When `ENTRY` ends in `.wasm`, `build` validates the Component Model task ABI
and imports, retains the portable Component, and may include compatible
host-specific optimization metadata. The resulting executable remains a
one-shot JSON task rather than an HTTP server. See
[Wasm Component runtime](../component/runtime.md#portable-component-contract).

```sh
tysel build --release --output dist/orders
```

## `tysel image`

Generate a non-root Linux container context and optionally ask Docker to build
it.

```text
tysel image [ENTRY] [OPTIONS]
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--binary <path>` | Not set | Copy an existing Linux Tysel executable instead of building the application. Its embedded application, profile, and listener must match the selected manifest. |
| `--stub <path>` | Bundled stub | Select the runtime stub used when building. |
| `--tag <name>` | `<app.name>:latest` | Build a tagged image after generating the context. |
| `--output-dir <path>` | `dist/image` | Destination for generated context files. |
| `--base-image <image>` | `gcr.io/distroless/cc-debian13:nonroot` | Runtime base image. |
| `--builder <path>` | `$DOCKER`, then `docker` | Container builder executable, for example `podman`. Embedded arguments are not supported. |
| `--copy-sidecars` | Off | With `--binary`, verify and copy its five release sidecars into the build context for CI admission. The recorded target must match the ELF architecture. They are not copied into the runtime image. |
| `--image-version <version>` | Not set | Set `org.opencontainers.image.version`. |
| `--label <KEY=VALUE>` | None | Add an image label. Repeat the option for multiple labels. Tysel-generated keys cannot be overridden. |
| `--context-only` | Off | Generate files without invoking Docker. |
| `--force` | Off | Replace generated context files. When sidecars are not copied, remove stale generated sidecars from the context. |
| `--manifest <file>` | Discovered | Select one manifest. |

```sh
tysel image --context-only
tysel image \
  --binary dist/orders \
  --copy-sidecars \
  --image-version 1.4.0 \
  --tag example/orders:1.4.0
```

Review generated files before publishing. The command's default image runs as
a non-root user, but registry authentication, image signing, and deployment
policy remain operator responsibilities.

Build behavior:

| Command | Builds the application | Runs `docker build` |
| --- | --- | --- |
| `tysel image` | Yes, in release mode | Yes |
| `tysel image --context-only` | Yes, in release mode | No |
| `tysel image --binary PATH` | No | Yes |
| `tysel image --binary PATH --context-only` | No | No |

On a non-Linux host, `--binary` is required. It must name a 64-bit
little-endian Linux x86-64 or arm64 Tysel executable with valid embedded TAP
metadata. The embedded application name, execution profile, and listener must
match the selected manifest. Container listeners must use `0.0.0.0` or `[::]`
and a non-zero port. The `component` profile is rejected; use
[Component tasks](../../operations/component-tasks.md).

Every generated Dockerfile includes `io.tysel.artifact.digest`,
`io.tysel.execution-profile`, `io.tysel.runtime.version`, and
`org.opencontainers.image.title`. These labels contain identifiers, not
secrets. The artifact digest describes the packaged executable; it is not the
registry image digest.
Custom label values are preserved literally, including `$` characters.

The default base-image check requires either a glibc interpreter or a static
ELF. When `--base-image` selects another image, Tysel validates the Linux ELF,
supported architecture, and interpreter structure but does not claim that the
custom image contains the required libc or shared libraries.

See the end-to-end
[Container image reference](../../guides/container-image.md),
[Continuous delivery](../../operations/continuous-delivery.md),
[Production operations](../../operations/production.md), and
[Benchmarks and release evidence](evidence.md).
