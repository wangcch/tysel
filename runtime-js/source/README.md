# Authored runtime sources

These files are the maintainable inputs for the embedded `runtime.js` artifacts.
Run `pnpm --filter @tysel/runtime-js build:runtime` after editing them. The
repository check uses `--check` and fails when a generated artifact is stale.

Web API and capability-client sources are independently scoped scripts. Durable
sources are ordered `.part.js` fragments because they deliberately share one
replay-state closure; they are syntax-checked after deterministic assembly.
