# Wasm Components

Status: **experimental**. Tysel runs language-neutral, one-shot tasks through
the WebAssembly Component Model. It does not execute arbitrary Core WebAssembly
modules or provide a general WASI application host.

## Product contract

A Component application has three defining properties:

1. its manifest uses `profile = "component"` and a `.wasm` entry;
2. the entry implements `tysel:component/task@0.4.0`;
3. one JSON value enters on stdin and one JSON value leaves on stdout.

```toml
[app]
name = "echo-component"
entry = "echo.component.wasm"
profile = "component"
```

The Component profile is useful for bounded functions written in Rust, Go, or
another language with Component Model tooling. It is intentionally smaller
than Tysel's TypeScript service runtime.

## Command behavior

| Command | Component behavior |
| --- | --- |
| `tysel check` | Validates the Component binary, task ABI, WASI imports, capability imports, and manifest grants. TypeScript checking and Node scanning do not apply. |
| `tysel dev` | Fails. Components are one-shot tasks and do not run as reloadable HTTP services. |
| `tysel run` | Reads one JSON value from stdin, invokes the Component once, writes one JSON value and a newline to stdout, then exits. |
| `tysel build` | Packages the portable Component and host-specific AOT metadata into one native executable. |
| Built executable | Uses the same one-shot stdin/stdout contract. Deployment capability policy is deny-by-default. |

```sh
printf '{"value":42}' | tysel run
tysel build
printf '{"value":42}' | ./dist/echo-component
```

## What is not provided

- no HTTP listener, Fetch handler, Cron, Queue, MCP, or durable handler;
- no `tysel dev` watch mode;
- no JavaScript globals or `globalThis.tysel` object;
- no ambient arguments, environment, inherited stdio, preopened directories,
  or network access through WASI;
- no general support for Core Wasm, WASI CLI applications, or arbitrary WIT
  worlds.

## Reference map

| Need | Reference |
| --- | --- |
| Exact task world and JSON boundary | [Component ABI](abi.md) |
| Wasmtime execution, restricted WASI, AOT behavior, and limits | [Runtime and WASI](runtime.md) |
| Filesystem imports and three-layer grants | [Component capabilities](capabilities.md) |
| Rust guest types and dispatcher | [Rust SDK](rust-sdk.md) |
| Go guest dispatcher and generated bindings | [Go SDK](go-sdk.md) |

For a runnable path, follow [Build a Rust Component](../../guides/wasm-component-rust.md)
or [Build a Go Component](../../guides/wasm-component-go.md). The installed
binary and the WIT files bundled with its source revision are authoritative.
