# Rust Component SDK

Status: **experimental and workspace-local**. `tysel-component-sdk` is not yet
published as a stable crates.io dependency. Use the SDK and WIT files from the
same Tysel source revision as the runtime.

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
    path: "../../../wit/component",
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

The repository fixture pins `wit-bindgen` `0.57.1` and builds a
`wasm32-unknown-unknown` core module before wrapping it as a Component with
`wasm-tools` `1.247.0`. These are verified fixture versions, not an open-ended
toolchain compatibility promise.

Follow [Build a Rust Component](../../guides/wasm-component-rust.md) for the
tested commands and [Component ABI](abi.md) for host validation.
