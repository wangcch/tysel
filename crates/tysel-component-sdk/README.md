# tysel-component-sdk

Rust guest-side JSON dispatch for the experimental
`tysel:component/task@0.4.0` Wasm Component interface.

```toml
[dependencies]
tysel-component-sdk = "0.1.1"
```

Implement one typed task:

```rust
use serde::{Deserialize, Serialize};
use tysel_component_sdk::{dispatch, Task};

#[derive(Deserialize)]
struct Input { value: u32 }

#[derive(Serialize)]
struct Output { doubled: u32 }

struct Double;

impl Task for Double {
    type Input = Input;
    type Output = Output;

    fn run(input: Input) -> Result<Output, String> {
        Ok(Output { doubled: input.value * 2 })
    }
}
```

Bridge it from the WIT-generated export:

```rust
impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        dispatch::<Double>(&input)
    }
}

export!(Component);
```

The dispatcher validates JSON and enforces the host's 1 MiB input/output and
4 KiB error limits. SDK `0.1.x` targets Tysel `0.1.x` and Component ABI `0.4.0`.

Use the complete [Rust Component guide](https://tysel.dev/docs/guides/wasm-component-rust/)
for bindings and build commands. See also the
[SDK reference](https://tysel.dev/reference/component/rust-sdk/).
