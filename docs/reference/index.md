# API reference

Reference pages describe exact interfaces: accepted values, defaults, side
effects, errors, and stability boundaries. The public website publishes this
section at `/docs/reference/` so contracts stay separate from guides. Start with a
[guide](../guides/index.md) when you want a workflow rather than a contract.

## Browse by surface

| Surface | Start here | Authoritative source |
| --- | --- | --- |
| JavaScript globals | [JavaScript APIs](javascript/index.md) | `runtime-js/web-api/compatibility.json` |
| Application exports and `tysel.*` | [Runtime](runtime/index.md) | `@tysel/types` and the runtime implementation |
| Wasm Component ABI, SDKs, and restricted WASI | [Wasm Components](component/index.md) | Versioned WIT, guest SDKs, and Wasmtime host implementation |
| Commands and flags | [CLI](cli/index.md) | Installed command definitions and `tysel <command> --help` |
| Manifest keys | [Manifest](manifest/index.md) | Bundled Draft 2020-12 JSON Schema |
| Host settings | [Environment variables](environment.md) | Installer, CLI, and host adapters |
| Defaults and hard bounds | [Limits and defaults](limits-and-defaults.md) | Schema defaults and native implementation constants |
| Exit codes and error envelopes | [Errors and machine output](errors-and-output.md) | CLI and HTTP response implementations |
| Grants by execution profile | [Capability matrix](../capabilities/README.md) | Manifest, profile, and host enforcement |
| npm or Node.js assumptions | [npm compatibility](../compatibility/README.md) | Compatibility catalog and project scan |

## Lookup by symbol

| Symbol or command | Reference |
| --- | --- |
| `fetch`, `Request`, `Headers`, `URL`, `crypto`, `WebSocket` | [JavaScript APIs](javascript/index.md) |
| `JsonValue`, `MaybePromise`, `ExecutionProfile`, `TrustMode` | [Core types](runtime/types.md) |
| `TyselApp`, `FetchHandler`, `RequestContext` | [Application module](runtime/application.md) |
| `tysel.secrets`, `tysel.sqlite`, `tysel.postgres`, `tysel.fs`, `tysel.llm` | [Host capabilities](runtime/capabilities.md) |
| `DurableContext`, `tysel.durable` | [Durable API](runtime/durable.md) |
| `test`, `assert`, `invokeFetch` | [Testing API](runtime/testing.md) |
| `tysel:component/task@0.4.0`, `tysel:fs/read@0.4.0`, `tysel:fs/write@0.4.0` | [Wasm Components](component/index.md) |
| `tysel init`, `tysel config` | [Project and configuration commands](cli/project.md) |
| `tysel check`, `compat`, `test`, `dev`, `run`, `inspect` | [Develop and test commands](cli/development.md) |
| `tysel task`, `queue`, `mcp` | [Tasks and protocols](cli/tasks.md) |
| `tysel build`, `image` | [Build and image](cli/delivery.md) |
| `tysel doctor`, `upgrade` | [Installation lifecycle](cli/installation.md) |
| `tysel bench`, `release` | [Benchmarks and release evidence](cli/evidence.md) |

## Version-local truth

The reference describes the source tree that produced this documentation. For
an installed binary, its help and bundled schema are authoritative:

```sh
tysel --version
tysel --help
tysel config schema
tysel config show --format json
tysel inspect
```

Unknown manifest fields are errors. APIs or environment variables not listed
here should be treated as implementation details unless a release note promotes
them to a public contract.
