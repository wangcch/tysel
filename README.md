# Tysel

> **Write TypeScript. Ship a binary.**

Tysel is a lightweight native TypeScript runtime for services and AI agents.
It packages your application and runtime into one executable—without Node.js,
V8, or `node_modules` in production.

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

## Why Tysel

- **Ship one file.** Deploy an executable instead of a JavaScript environment.
- **Grant capabilities explicitly.** Network, secrets, databases, and files are
  denied until declared.
- **Suspend and resume work.** Durable steps, effects, sleep, retry, and signals
  survive process restarts.
- **Run language-neutral tasks.** Package bounded Rust or Go Wasm Components
  behind a versioned JSON ABI and restricted WASI host.

Tysel uses Web-standard APIs and is built for HTTP services, workers, MCP tools,
isolated plugins, and durable agents.

## Install

Install the latest published release on Linux or macOS:

```sh
curl -fsSL https://tysel.dev/install.sh | sh
tysel doctor --install
```

The installer adds the complete native toolchain; cloning this repository,
Rust, Node.js, and npm are not required. See the [installation guide](docs/install.md)
for version pinning, upgrades, rollback, and Windows support.

## Try it

```sh
tysel init hello-tysel --yes
cd hello-tysel
tysel task verify
tysel dev
```

Then package the application:

```sh
tysel task release
./dist/hello-tysel
```

The developer toolchain uses three binaries. Your application ships as one.

## Scope

Tysel is Web-API-first, not a general Node.js compatibility layer. Node
builtins, native addons, subprocesses, dynamic libraries, and ambient host
access are outside its application contract.

The `service` profile is for trusted application code. The `isolated` profile
runs JavaScript in a separate worker process; Linux is the production isolation
target. The experimental `component` profile runs one-shot Wasm Component
tasks with no ambient host authority. Cross-compilation is not currently
implemented.

## Learn more

- [Getting started](docs/getting-started.md)
- [How Tysel works](docs/concepts/how-tysel-works.md)
- [Projects and configuration](docs/concepts/projects-and-configuration.md)
- [Wasm Components](docs/reference/component/index.md)
- [Documentation](docs/index.md)

## License

[Apache-2.0](LICENSE)
