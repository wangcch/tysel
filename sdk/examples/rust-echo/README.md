# Rust echo Component

Prerequisites: stable Rust and Bytecode Alliance `wasm-tools` 1.247.0.

```sh
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
wasm-tools component new \
  target/wasm32-unknown-unknown/release/tysel_rust_echo_component.wasm \
  -o target/echo.component.wasm
```

The generated Component has no implicit WASI imports, implements the repository's
`tysel:component/task@0.4.0` world and can be packaged as the `entry` in
`tysel.toml`.
