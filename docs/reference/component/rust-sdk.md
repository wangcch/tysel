# Rust Component SDK

Status: **experimental and versioned**. For a Rust Component project, use the
published crate that matches the installed Tysel minor release:

```toml
tysel-component-sdk = "0.1.0"
```

The release starter is also self-contained: it vendors the same SDK version and
uses `tysel-component-sdk = { path = "sdk/tysel-component-sdk" }`. Use the
crates.io dependency for your own project, or keep the vendored path dependency
when reproducible or offline starter builds are more important.

## Guest interface

```rust
pub trait Task {
    type Input: serde::de::DeserializeOwned;
    type Output: serde::Serialize;

    fn run(input: Self::Input) -> Result<Self::Output, String>;
}

pub fn dispatch<T: Task>(input: &str) -> Result<String, String>;
```

`dispatch` validates and decodes JSON, calls `Task::run`, serializes the result,
and enforces 1 MiB input, 1 MiB output, and 4 KiB error bounds.

## WIT binding

The SDK does not generate bindings. A guest uses `wit-bindgen` against
`wit/component` and connects the generated `Guest::run` to `dispatch`:

```rust
use serde::{Deserialize, Serialize};
use tysel_component_sdk::{dispatch, Task};

wit_bindgen::generate!({
    path: "wit/component",
    world: "task",
});

#[derive(Deserialize)]
struct Input { value: serde_json::Value }

#[derive(Serialize)]
struct Output { value: serde_json::Value }

struct Echo;

impl Task for Echo {
    type Input = Input;
    type Output = Output;

    fn run(input: Input) -> Result<Output, String> {
        Ok(Output { value: input.value })
    }
}

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        dispatch::<Echo>(&input)
    }
}

export!(Component);
```

The release starter pins `wit-bindgen` `0.57.1` and builds a
`wasm32-unknown-unknown` core module before wrapping it as a Component with
`wasm-tools` `1.247.0`. These are verified fixture versions, not an open-ended
toolchain compatibility promise.

Follow [Build a Rust Component](../../guides/wasm-component-rust.md) for the
tested commands and [Component ABI](abi.md) for host validation.
