# Tysel

> **A lightweight native runtime for TypeScript services and agents.**
>
> **Write TypeScript. Ship a binary.**

Tysel runs TypeScript services, workers, and agents as a single native executable. Production does not require Node, V8, or `node_modules`.

The public API prefers Web standards (`Request`, `Response`, `fetch`, streams, `crypto`). Platform capabilities are granted explicitly, not through ambient Node modules.

This repository is in **M1**. `tysel check` validates a project; `tysel dev` serves with file-watch reload; `tysel build` emits a single native executable. The full plan is in [roadmap.md](./roadmap.md).

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

`tysel build` copies a `tysel-service` stub and appends a TAP trailer. It looks for the stub in `--stub`, `TYSEL_STUB`, next to the `tysel` binary, `target/release` or `target/debug`, then `PATH`. `--target` must match this host; cross-compilation is not implemented. `--release` searches for a release stub. The command type-checks when TypeScript is present, then prints bundle size, capabilities, and the output path.

```bash
cargo build -p tysel-runtime --bin tysel-service --release
cargo run -p tysel-cli -- build --manifest examples/hello-service/tysel.toml
```

## Commands

```bash
cargo run -p tysel-cli -- check --manifest tysel.toml
cargo run -p tysel-cli -- dev --manifest tysel.toml
cargo run -p tysel-cli -- inspect --manifest tysel.toml
cargo run -p tysel-cli -- build --manifest tysel.toml
```

`tysel check` loads the manifest, bundles the entry, and runs `tsc --noEmit` when a `tsconfig.json` and TypeScript are present. Missing TypeScript is skipped, not a failure.

`tysel dev` serves the bundled app, prints `tysel listen <addr>`, and reloads isolates when `ts` / `js` / `json` / `toml` files change. It does not watch `node_modules`, `target`, `dist`, `.git`, or `data`. Reload keeps the same port; keep-alive connections pick up the new isolate on the next request.

Trusted-path `fetch` supports HTTP and HTTPS GET, HEAD, POST, PUT, PATCH, and DELETE. Hosts must be listed in `[permissions] fetch`; an empty list denies every outbound request. Header values that are `secret:name` or `Bearer secret:name` are expanded in the host and never returned to JavaScript. String bodies are sent as-is (GET/HEAD ignore a body) and are capped at 16MiB. The returned `Response` exposes origin headers (hop-by-hop headers omitted). Redirects are followed (max 20) and isolate timeout and cancel are honored. `tysel.httpGet(url)` is a GET wrapper.

When `[app] profile = "isolated"`, outbound fetch, SQLite, WebSocket, Postgres, and filesystem access are denied even if listed in `[permissions]`. `tysel.sleep`, `tysel.echo`, and `tysel.secrets.ref` remain available. `tysel dev` and a packaged stub run the fetch handler in a `tysel-worker` child process (set `TYSEL_WORKER` or place the binary next to `tysel`). Request and response bodies over the worker pipe are capped at 32KiB. The supervisor keeps secret values; the worker only sees declared names. Isolated bundles must fit in a 64KiB IPC frame. On Linux the worker also applies Landlock (no host files except `/dev/urandom` / `/dev/random`) and a seccomp allowlist (no exec, sockets, ptrace, mount, or bpf). The supervisor best-effort attaches the worker to a cgroup v2 `memory.max` when the host allows it. macOS is not that security gate.

`setTimeout` / `setInterval` run while the current request or eval is pending; leftover timers are dropped when the request ends. `TextEncoder` / `TextDecoder` are UTF-8 only. `crypto.getRandomValues` fills at most 65536 bytes. `crypto.subtle` is not implemented yet.

Inbound WebSocket is available on the trusted path when `[server] websocket = true`. A handler calls `tysel.acceptWebSocket()`, returns status 101, and can `send` / `addEventListener("message")` for text frames. Isolated workers cannot accept WebSockets. Outbound `WebSocket` clients are not implemented yet.

Trusted-path SQLite is available as `tysel.sqlite.exec(sql, params?)` and `tysel.sqlite.query(sql, params?)`. Parameters are bound (never concatenated). Isolated workers cannot open SQLite. The default database is in-memory; `[durable] store = "sqlite"` with `path` pins a file (created on first use). `tysel dev` resolves a relative path against the manifest directory; a packaged binary resolves it against the process working directory. See `examples/sqlite-worker`.

Trusted-path Postgres is available as `tysel.postgres.exec(sql, params?)` and `tysel.postgres.query(sql, params?)` when `[permissions] postgres` lists a named connection such as `main:read-write`. Connection URLs must not appear in the manifest or TAP trailer; the host reads `TYSEL_POSTGRES_<NAME>` from the process environment (`tysel dev` also reads a sibling `.env`). Placeholders are `$1`, `$2`, … (not SQLite `?`). JSON integers are encoded to match the target column (INT2/INT4/INT8). Queries stream rows and stop at 10,000 rows or 1MiB of result payload. Isolated workers cannot open Postgres. TLS and connection pooling are not implemented yet. See `examples/postgres-service`.

Trusted-path filesystem access is available as `tysel.fs.read(path)` and `tysel.fs.write(path, data)` when `[permissions] fs_read` / `fs_write` list directory roots. Relative roots and relative request paths are resolved against the manifest directory in `tysel dev`, and against the process working directory in a packaged binary. Paths are opened relative to the root directory fd (`openat`, and `openat2` with `RESOLVE_BENEATH` on Linux); `..` and symlinks cannot escape the allowlist. Reads and writes are capped at 1MiB UTF-8. Unconfigured processes deny every path. Isolated workers cannot use the filesystem.

Trusted-path secrets are opaque handles: `tysel.secrets.ref("OPENAI_API_KEY")` returns `secret:OPENAI_API_KEY` and never the raw value. Names come from `[permissions] secrets`; values are loaded from the process environment, and `tysel dev` also reads a sibling `.env` for those names only. `tysel dev` reloads declared secrets when `tysel.toml` or `.env` changes. Isolated workers can mint handles through the supervisor broker but cannot read raw secrets.

When `[observability] logs = "json"` (the default), each HTTP request writes one JSON line to stderr with `ts`, `app`, `method`, `path`, `status`, and `ms`. Query strings and headers are omitted. Set `logs` to any other value to disable.

Isolate hot-swap and `tysel run` are not implemented yet.

## License

Apache-2.0
