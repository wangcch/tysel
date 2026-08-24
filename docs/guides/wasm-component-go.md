# Build a Go Wasm Component

This guide generates the repository's Go bindings, builds a Wasm Component,
validates its ABI, and packages it as a native Tysel executable.

## Prerequisites

Use one repository checkout so the experimental runtime, WIT, SDK, and
generated bindings share a source revision. Install:

- the Tysel [source-build prerequisites](../install.md#current-availability);
- Go `1.25.5`, matching the fixture module;
- Bytecode Alliance `componentize-go` `0.4.1`.

```sh
go install github.com/bytecodealliance/componentize-go@v0.4.1
cargo build --locked --release \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service
export PATH="$PWD/target/release:$PATH"
```

Ensure the Go installation's binary directory is also on `PATH` so
`componentize-go` is available.

## Generate and build

From the repository root:

```sh
cd sdk/examples/go-echo
componentize-go -d ../../../wit/component -w task bindings
componentize-go -d ../../../wit/component -w task build \
  -o echo.component.wasm
```

The repository commits `wit_exports.go`, so regenerating bindings is useful
when verifying the pinned toolchain or changing WIT. An ordinary fixture build
can use the committed bindings.

## Validate and run

```sh
tysel check
printf '{"language":"go","value":42}' | tysel run
```

Expected stdout:

```json
{"language":"go","value":42}
```

The guest receives and returns JSON through
`tysel:component/task@0.4.0`. Tysel owns process stdin/stdout; the guest does
not inherit them as ambient WASI streams.

## Package one executable

```sh
tysel build --release
printf '{"value":"packaged"}' | ./dist/go-echo-component
```

The executable retains portable Component source for compatibility and embeds
host-specific AOT metadata. It runs without Go or `componentize-go` on the
target host.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Generated package does not compile | Use `componentize-go` `0.4.1` with the repository's `wit/component` directory. |
| Invalid output JSON | Return a value encodable by `encoding/json`; do not construct the ABI string manually. |
| Process waits without output | Close stdin after writing the single JSON input. |
| WASI socket or preopen import rejected | Tysel exposes only its [restricted WASI profile](../reference/component/runtime.md#restricted-wasi-02-profile). |
| Filesystem import denied | Satisfy the [Component capability intersection](../reference/component/capabilities.md). |

Continue with the [Go SDK reference](../reference/component/go-sdk.md).
