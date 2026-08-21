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
   versions fail closed. M4 ships `tysel:fs/read@0.4.0` and
   `tysel:fs/write@0.4.0`; their string boundary carries JSON and the existing
   confined filesystem implementation enforces resource allowlists. Read and
   write roots are unique and capped at 64 per operation before directory fds
   are opened. Host filesystem calls run through a fixed four-worker,
   32-request executor and observe the Component execution deadline.
   Deployment policy can approve `tysel:fs/read` and `tysel:fs/write`
   independently; the broader `tysel:fs` grant remains a compatibility alias.
   Every linked FS call emits an application-scoped metadata-only audit event
   when JSON logging is enabled, without recording paths or file contents.
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

`tysel run` executes a Component as a one-shot stdin/stdout JSON task. Local
execution treats manifest-declared filesystem access as its deployment policy
and resolves relative roots from the manifest directory. Packaged executables
remain fail-closed unless `TYSEL_COMPONENT_CAPABILITIES` explicitly contains
`tysel:fs`, `tysel:fs/read`, or `tysel:fs/write` (a comma-separated allowlist).
`tysel dev` rejects Component entries because HTTP reload semantics do not
apply to one-shot tasks.

The local run path packages portable source only. Release packaging still
produces AOT admission metadata, while development avoids generating an AOT
blob that unsigned execution must intentionally ignore.
