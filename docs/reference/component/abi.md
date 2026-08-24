# Component ABI

Tysel's public Component entrypoint is the versioned world
`tysel:component/task@0.4.0`.

## WIT contract

```wit
package tysel:component@0.4.0;

world task {
    export run: func(input: string) -> result<string, string>;
}
```

Source of truth: `wit/component/task.wit`.

Both the input string and successful output string must contain one valid UTF-8
JSON value. The host validates input before instantiation and validates output
after the guest's canonical ABI cleanup. JSON can be an object, array, string,
number, boolean, or `null`.

The error branch is a bounded human diagnostic string; it is not required to
contain JSON.

## Required export shape

The Component must export a top-level function named `run` with exactly the
parameters and result above. Validation rejects:

- a Core WebAssembly module instead of a Component binary;
- a Component without `run`;
- additional or different `run` parameters and results;
- an incompatible Component ABI version;
- malformed or oversized JSON at the host boundary.

File extension alone is not validation. A `.wasm` path selects the Component
build path, after which Tysel parses and validates the Component Model binary.

## Version compatibility

| Contract | Current value | Compatibility rule |
| --- | --- | --- |
| Component task ABI | `0.4.0` | The packaged component must match the runtime ABI version. |
| Filesystem capability ABI | `0.4.0` | Only the exact implemented imports are admitted. |
| Restricted WASI imports | `0.2.x` | Only the allowlisted language-runtime interfaces are accepted. |

The Component ABI is experimental. Treat a version change as a rebuild
boundary for the guest and packaged application. Tysel retains portable source
in the executable, but portable source does not make incompatible WIT exports
or imports compatible.

## SDK dispatch

The Rust and Go SDK helpers apply the same JSON and byte bounds before data
crosses the canonical ABI:

```text
JSON string → decode → typed/raw handler → encode → JSON string
```

They do not generate WIT bindings. Binding generation remains specific to the
guest language and toolchain. See the [Rust SDK](rust-sdk.md) and
[Go SDK](go-sdk.md).
