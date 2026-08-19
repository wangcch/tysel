# Tysel

> **A lightweight native runtime for TypeScript services and agents.**
>
> **Write TypeScript. Ship a binary.**

Tysel runs TypeScript services, workers, and agents as a single native executable. Production does not require Node, V8, or `node_modules`.

The public API prefers Web standards (`Request`, `Response`, `fetch`, streams, `crypto`). Platform capabilities are granted explicitly, not through ambient Node modules.

This repository is in **M1**. `tysel check` validates a project; `tysel dev` serves with file-watch reload. The full plan is in [roadmap.md](./roadmap.md).

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
cargo run -p tysel-cli -- check --manifest examples/hello-service/tysel.toml
cargo run -p tysel-cli -- dev --manifest examples/hello-service/tysel.toml
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

`tysel build` emits a single native executable when a `tysel-service` stub is on `PATH` or passed as `--stub`.

## Commands

```bash
cargo run -p tysel-cli -- check --manifest tysel.toml
cargo run -p tysel-cli -- dev --manifest tysel.toml
cargo run -p tysel-cli -- inspect --manifest tysel.toml
cargo run -p tysel-cli -- build --manifest tysel.toml --stub /path/to/tysel-service
```

`tysel check` loads the manifest, bundles the entry, and runs `tsc --noEmit` when a `tsconfig.json` and TypeScript are present. Missing TypeScript is skipped, not a failure.

`tysel dev` serves the bundled app, prints `tysel listen <addr>`, and reloads isolates when `ts` / `js` / `json` / `toml` files change. It does not watch `node_modules`, `target`, `dist`, `.git`, or `data`. Reload keeps the same port; keep-alive connections pick up the new isolate on the next request.

Trusted-path `fetch` supports HTTP and HTTPS GET/HEAD, follows redirects (max 20), and honors isolate timeout and cancel. `tysel.httpGet(url)` is a GET wrapper. Isolated workers cannot open outbound HTTP. Request bodies are not implemented yet.

Isolate hot-swap and `tysel run` are not implemented yet.

## License

Apache-2.0
