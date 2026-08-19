# Tysel

> **A lightweight native runtime for TypeScript services and agents.**
>
> **Write TypeScript. Ship a binary.**

Tysel runs TypeScript services, workers, and agents as a single native executable. Production does not require Node, V8, or `node_modules`.

The public API prefers Web standards (`Request`, `Response`, `fetch`, streams, `crypto`). Platform capabilities are granted explicitly, not through ambient Node modules.

This repository is in **M0**. The workspace, CLI, manifest schema, and SDK types are in place. Spike A (QuickJS-ng + native async) has landed; HTTP service, single-file packaging, and isolated workers are next. The full plan is in [roadmap.md](./roadmap.md).

## Layout

```text
crates/          Rust workspace (runtime core, CLI, capabilities)
packages/        TypeScript SDK and shared types
runtime-js/      Isolate bootstrap, Web API, durable client
wit/             Capability WIT ABI (experimental)
examples/        Sample applications
docs/adr/        Architecture decisions
benchmarks/      Performance harnesses
```

## Requirements

- Rust 1.85+ (`rustup` stable)
- Node.js 22+
- pnpm 11+
- TypeScript 7 CLI (`tsc --noEmit`; the compiler is not embedded)

## Quick start

```bash
pnpm install
cargo test --workspace
cargo run -p tysel-cli -- --help
cargo run -p tysel-cli -- inspect --manifest examples/hello-service/tysel.toml
```

Minimal application:

```ts
export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
};
```

`tysel dev` and `tysel build` land after the remaining M0 spikes.

## License

Apache-2.0
