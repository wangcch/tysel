# tysel-component-sdk

Rust guest-side helpers for the experimental
`tysel:component/task@0.4.0` Wasm Component interface.

```rust
use serde::{Deserialize, Serialize};
use tysel_component_sdk::{Task, dispatch};

#[derive(Deserialize)]
struct Input {
    value: u32,
}

#[derive(Serialize)]
struct Output {
    doubled: u32,
}

struct Double;

impl Task for Double {
    type Input = Input;
    type Output = Output;

    fn run(input: Input) -> Result<Output, String> {
        Ok(Output { doubled: input.value * 2 })
    }
}
```

Call `dispatch::<Double>(input)` from the WIT-generated `Guest::run`
implementation. The helper validates JSON and enforces the same 1 MiB
input/output and 4 KiB error limits as the host.

See the [Rust echo Component](../../sdk/examples/rust-echo/README.md) and the
[Component SDK overview](../../sdk/README.md).
