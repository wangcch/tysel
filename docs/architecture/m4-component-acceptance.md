# M4 Wasm Component v0.4 acceptance

M4 is complete when the following behavior is implemented and covered by the
workspace CI matrix on macOS arm64, Linux x86-64, and Linux arm64.

1. Wasmtime executes Component Model binaries with no application capabilities
   by default. A fixed WASI 0.2 language-runtime profile may be linked with
   closed stdio, empty arguments/environment/preopens, and denied networking.
   Component bytes, linear memory, execution fuel, inputs, outputs, instances,
   memories, and tables are bounded.
2. `tysel:component/task@0.4.0` provides the versioned JSON-string boundary.
   ABI-major incompatibilities and unknown imports fail before guest code runs.
3. The Capability Registry deterministically links only manifest-declared and
   deployment-approved WIT imports. Duplicate registrations and incompatible
   versions fail closed.
4. Rust and Go SDK examples implement the same WIT world and pass host-side
   contract, error, limit, and cross-language fixture tests.
5. AOT artifacts carry target, Wasmtime compatibility identity, source digest,
   and ABI identity. Stale or cross-target artifacts are rejected safely.
6. TAP packages embed bounded components and their metadata; development and
   packaged execution preserve the same policy and failure semantics. Isolated
   Component workers remain a production-hardening item after the M4 engine
   and packaging boundary.

The M4 implementation keeps portable Component source as the safe fallback.
AOT bytes are admitted only when target, Wasmtime engine identity, ABI, source
digest, and size all match. Native AOT deserialization remains disabled until
TAP signatures make unsafe native-code loading an authenticated boundary.
