# Getting started

This guide creates, tests, runs, and packages a small HTTP service.

## Prerequisites

Tysel has not published a binary release yet. Build all three tools from source
and add their directory to `PATH`:

```sh
git clone https://github.com/wangcch/tysel.git
cd tysel
pnpm install
cargo build --locked --release \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service \
  -p tysel-isolate --bin tysel-worker
export PATH="$PWD/target/release:$PATH"
tysel doctor
```

See [Installation](install.md) for prerequisites and the managed install,
upgrade, and rollback contract that activates after a tagged release exists.

## Create a project

```sh
cd ..
tysel init hello-tysel
cd hello-tysel
```

In an interactive terminal, `init` offers a recommended quick start and a
customized flow. Customize can select an HTTP service, queue worker, MCP tool,
or minimal template, along with manifest/package/test choices. Use `--yes` to
accept explicit choices without prompting or `--no-interactive` in automation;
every prompt also has a corresponding command-line option.

`init` creates:

```text
hello-tysel/
├── src/index.ts
├── tests/app.test.ts
├── package.json
├── tsconfig.json
└── tysel.toml
```

It checks all destinations before writing and does not overwrite existing
files.

Choose JSON explicitly, preview changes, or omit a newly generated
`package.json`:

```sh
tysel init hello-json --manifest-format json
tysel init hello-native --package-json none --dry-run
tysel init hello-mcp --template mcp --package-json none --no-tests --yes
```

Inside an existing Node project, `init` preserves `package.json` and existing
source files and creates a separate `src/tysel.ts` entry and
`tsconfig.tysel.json` by default. The Tysel-specific config keeps application
checking isolated from the existing Node compiler configuration:

```sh
tysel init .
tysel init . --add-scripts # optionally add tysel:* package scripts
```

With `--no-tests`, Init also omits test dependencies, the `test` package script,
and the `test` step from the generated `verify` task.

The generated project pins `@tysel/types` and `@tysel/test` to the native
toolchain version. Those npm packages are not public yet, so do not install the
generated dependencies from the registry. Runtime and test commands work
without them; `tysel check` reports that TypeScript checking was skipped.

To enable editor and compiler declarations during source development, build
and add the local packages from the adjacent Tysel checkout:

```sh
pnpm --dir ../tysel --filter @tysel/types build
pnpm --dir ../tysel --filter @tysel/test build
pnpm add -D \
  ../tysel/packages/tysel-types \
  ../tysel/packages/tysel-test \
  typescript@7.0.2
```

## Write a Fetch handler

The generated entry exports a Fetch-style application:

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

Validate the manifest, TypeScript, capabilities, and imports:

```sh
tysel check
tysel compat
```

`compat` is an early dependency report, not a substitute for tests. See
[npm compatibility](compatibility/README.md).

## Test and run

```sh
tysel test
tysel dev
```

The generated manifest also includes native `verify` and `release` workflows:

```sh
tysel task verify
tysel task release
```

`dev` watches the application and reloads it after source changes. Call the
address printed by the server:

```sh
curl http://127.0.0.1:3000/hello
```

Test files ending in `.test.ts`, `.test.mts`, `.test.js`, or `.test.mjs` are
discovered recursively under `tests/`. Each test runs in a fresh QuickJS
isolate. The built-in contract includes `test`, `assert`, `assert.equal`, and
`assert.deepEqual`.

## Grant a capability

Outbound resources are denied until declared. For example:

```toml
[permissions]
fetch = ["api.example.com"]
secrets = ["API_TOKEN"]
```

The manifest requests authority; the execution profile and deployment policy
can reduce it further. Review the [manifest reference](reference/manifest.md)
and [capability matrix](capabilities/README.md).

## Build one executable

```sh
tysel build --release
./dist/hello-tysel
```

The output contains the application and runtime. The target machine does not
need Node.js or `node_modules`. Builds currently target the host platform;
cross-compilation is not implemented.

For a container, set the listener to `0.0.0.0:3000` and use `tysel image` on
Linux. See [production operations](operations/production.md) before deploying.

## Use in CI

```sh
tysel --error-format json check
tysel compat --json --strict
tysel test --json
tysel build --release
```

Fatal JSON errors are written to stderr. Command reports such as test and
compatibility results are written to stdout.
