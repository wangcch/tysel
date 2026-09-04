# Build a Rust Wasm Component

This guide builds a Rust guest from the published Tysel Component starter,
validates its ABI, runs one JSON task, and packages it as a native executable.

## Prerequisites

Install:

- the published Tysel [toolchain](../install.md);
- stable Rust with the `wasm32-unknown-unknown` target;
- Bytecode Alliance `wasm-tools` `1.247.0`.

```sh
cargo install wasm-tools --version 1.247.0 --locked
rustup target add wasm32-unknown-unknown
tysel doctor --install
```

## Download the starter

Each Tysel GitHub Release includes a version-matched starter bundle. Download
and verify it, then enter the Rust project:

```sh
version="$(tysel --version | awk '{print $2}')"
release="https://github.com/wangcch/tysel/releases/download/v${version}"
curl -fsSLO "${release}/tysel-component-starters.tar.gz"
curl -fsSLO "${release}/tysel-component-starters.tar.gz.sha256"
curl -fsSLO "${release}/tysel-component-starters.tar.gz.sig.json"
expected="$(tr -d '[:space:]' < tysel-component-starters.tar.gz.sha256)"
actual="$(shasum -a 256 tysel-component-starters.tar.gz | awk '{print $1}')"
test "$actual" = "$expected"
tysel release verify-metadata \
  tysel-component-starters.tar.gz \
  tysel-component-starters.tar.gz.sig.json \
  --trust "${TYSEL_HOME:-$HOME/.tysel}/trust.json"
tar -xzf tysel-component-starters.tar.gz
cd tysel-component-starters/rust-echo
```

The immutable tag comes from the installed CLI, so the bundle and native
toolchain stay on the same release. The bundle vendors the matching
`tysel-component-sdk` source and Component WIT package, and pins the tested
`wit-bindgen` version. It does not require a Tysel repository checkout.
If Tysel was installed under a custom managed prefix without `TYSEL_HOME`, pass
that prefix's `trust.json` to `--trust`.

For a project created without the starter, depend on the public SDK crate and
copy the matching WIT package into the project:

```toml
tysel-component-sdk = "0.2.0"
```

The crate version follows the Tysel product release. The WIT contract keeps its
independent `tysel:component/task@0.4.0` ABI version.

## Build the guest

From `tysel-component-starters/rust-echo`:

```sh
cargo build --target wasm32-unknown-unknown --release
wasm-tools component new \
  target/wasm32-unknown-unknown/release/tysel_rust_echo_component.wasm \
  -o target/echo.component.wasm
```

The first command produces a Core Wasm module. `wasm-tools component new`
wraps it as the Component Model binary Tysel accepts. Passing the Core module
directly to Tysel fails validation.

## Validate and run

The starter manifest selects `profile = "component"` and the generated entry.

```sh
tysel check
printf '{"value":42}' | tysel run
```

Expected stdout from `run`:

```json
{"value":42}
```

`run` waits for stdin EOF, invokes the Component once, prints one JSON value,
and exits. `tysel dev` is not available for Component applications.

## Package one executable

```sh
tysel build --release
printf '{"value":"packaged"}' | ./dist/rust-echo-component
```

The output executable includes the portable Component and native host, and may
include compatible host-specific optimization metadata. It does not need Rust,
Go, a separately installed runtime engine, or the guest SDK on the target host.
Builds still target the build host.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `run` export or ABI error | Use the WIT package shipped with the matching starter and world `task`. |
| Core module rejected | Run `wasm-tools component new` and point the manifest at the resulting Component. |
| Input JSON error | Send exactly one JSON value and close stdin. |
| Component exceeds a limit | Check [Component runtime limits](../reference/component/runtime.md#resource-bounds). |
| Capability import rejected | Only the documented [filesystem imports](../reference/component/capabilities.md) are implemented. |

Continue with the [Rust SDK reference](../reference/component/rust-sdk.md) to
replace the echo handler with a typed task.
