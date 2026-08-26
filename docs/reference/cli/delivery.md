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
| `--binary <path>` | Build the app | Package an existing Tysel executable. |
| `--stub <path>` | Bundled stub | Select the runtime stub used when building. |
| `--tag <name>` | `<app.name>:latest` | Build a tagged image after generating the context. |
| `--output-dir <path>` | `dist/image` | Destination for generated context files. |
| `--base-image <image>` | `gcr.io/distroless/cc-debian13:nonroot` | Runtime base image. |
| `--context-only` | Off | Generate files without invoking Docker. |
| `--force` | Off | Replace generated context files where supported. |
| `--manifest <file>` | Discovered | Select one manifest. |

```sh
tysel image --context-only
tysel image --binary dist/orders --tag example/orders:local
```

Review generated files before publishing. The command's default image runs as
a non-root user, but registry authentication, image signing, and deployment
policy remain operator responsibilities.

On a non-Linux host, `--binary` is required and must name a 64-bit
little-endian Linux x86-64 or arm64 ELF. Container listeners must use
`0.0.0.0` or `[::]` and a non-zero port. See the end-to-end
[Container image guide](../../guides/container-image.md),
[Production operations](../../operations/production.md), and
[Benchmarks and release evidence](evidence.md).
