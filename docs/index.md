# Tysel documentation

Tysel is a lightweight native TypeScript runtime for services and AI agents.
Write against Web APIs, grant host capabilities explicitly, and ship one
executable without Node.js or `node_modules` in production.

## Start here

| Your goal | Start with | Then read |
| --- | --- | --- |
| Build an HTTP service | [Create your first service](getting-started.md) | [Runtime API](reference/runtime/index.md) |
| Add Tysel to an existing project | [Create or adopt a project](concepts/projects-and-configuration.md#create-or-adopt-a-project) | [npm compatibility](compatibility/README.md) |
| Run generated or third-party code | [Choose an execution profile](concepts/execution-profiles.md) | [Security model](security/README.md) |
| Build a Rust or Go Wasm task | [Wasm Component guides](guides/wasm-component-rust.md) | [Component reference](reference/component/index.md) |
| Build work that survives restarts | [Durable execution](concepts/durable-execution.md) | [Durable agent example](https://github.com/wangcch/tysel/tree/main/examples/durable-agent) |
| Evaluate Tysel for production | [Production operations](operations/production.md) | [Performance evidence](performance/README.md) |

Browse the [task-oriented guide map](guides/index.md), the
[runnable example gallery](guides/examples.md), or the
[API reference](reference/index.md) when you already know what you need.

## Five-minute path

Tysel has not published a tagged binary release yet, so first follow the
[source installation](install.md#current-availability). Then create and run a
small service:

```sh
tysel init hello-tysel --yes
cd hello-tysel
tysel task verify
tysel dev
```

Call the address printed by the server, then package the application:

```sh
curl http://127.0.0.1:3000/hello
tysel task release
./dist/hello-tysel
```

The developer installation contains three cooperating tools. The application
artifact is still one executable.

## At a glance

| Area | What Tysel provides | Primary command or API |
| --- | --- | --- |
| Develop | Project creation, validation, tests, and reload | `tysel init`, `check`, `test`, `dev` |
| Deliver | One native application executable and release evidence | `tysel build --release` |
| Bound authority | Explicit network, secret, database, and filesystem grants | `tysel inspect` and the manifest |
| Run services | Fetch handlers, Web APIs, HTTP, and WebSocket | `export default { fetch() {} }` |
| Run tasks | Cron, Queue, and MCP handlers on one bounded task model | `tasks` export |
| Run Wasm Components | Language-neutral, one-shot JSON tasks with restricted WASI | `profile = "component"` |
| Resume work | Persisted steps, effects, sleep, retry, and signals | `durable` export |

## Understand the contract

Tysel is Web-API-first, not a general Node.js compatibility layer. Node
builtins, native addons, subprocesses, dynamic libraries, and ambient host
access are outside the application contract. The `service` profile is for
trusted first-party code. The `isolated` profile uses a separate worker
process; Linux is its production security target. Cross-compilation is not
currently implemented.

Read [how Tysel works](concepts/how-tysel-works.md), browse the
[JavaScript API reference](reference/javascript/index.md), and
run [`tysel compat`](compatibility/README.md) before adopting a dependency.

## Explore by area

- **Learn:** [how Tysel works](concepts/how-tysel-works.md),
  [projects and configuration](concepts/projects-and-configuration.md),
  [execution profiles](concepts/execution-profiles.md), and
  [durable execution](concepts/durable-execution.md).
- **Build:** [runtime API](reference/runtime/index.md),
  [Wasm Components](reference/component/index.md),
  [capability matrix](capabilities/README.md), and
  [runnable examples](guides/examples.md).
- **Look up:** [API reference](reference/index.md),
  [CLI](reference/cli/index.md), [manifest](reference/manifest/index.md),
  [compatibility](compatibility/README.md), and
  [JavaScript APIs](reference/javascript/index.md).
- **Operate:** [installation and upgrades](install.md),
  [security](security/README.md),
  [production operations](operations/production.md), and
  [performance evidence](performance/README.md).

For tools that consume documentation, use the full [`llms.txt`](llms.txt) or
the compact [`llms-small.txt`](llms-small.txt) index.
