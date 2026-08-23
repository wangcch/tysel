# Reference

Reference pages describe exact interfaces: accepted values, defaults, side
effects, errors, and stability boundaries. Start with a [guide](../guides/index.md)
when you want a workflow rather than a contract.

## Find a contract

| I need to look up… | Start here | Authoritative source |
| --- | --- | --- |
| A command or option | [CLI](cli/index.md) | Installed command definitions and `tysel <command> --help` |
| A manifest key | [Manifest](manifest/index.md) | Bundled Draft 2020-12 JSON Schema |
| An application export or TypeScript symbol | [Runtime](runtime/index.md) | `@tysel/types` and the runtime implementation |
| A required host setting | [Environment variables](environment.md) | Installer, CLI, and host adapters |
| A default or hard bound | [Limits and defaults](limits-and-defaults.md) | Schema defaults and native implementation constants |
| An exit code or error envelope | [Errors and machine output](errors-and-output.md) | CLI and HTTP response implementations |
| A browser-style JavaScript API | [JavaScript API compatibility](../architecture/javascript-runtime-compatibility.md) | Versioned compatibility inventory |
| A grant available to an execution profile | [Capability matrix](../capabilities/README.md) | Manifest, profile, and host enforcement |
| An npm or Node.js assumption | [npm compatibility](../compatibility/README.md) | Compatibility catalog and project scan |

## Lookup by symbol

| Symbol or command | Reference |
| --- | --- |
| `JsonValue`, `MaybePromise`, `ExecutionProfile`, `TrustMode` | [Core types](runtime/types.md) |
| `TyselApp`, `FetchHandler`, `RequestContext` | [Application module](runtime/application.md) |
| `tysel.secrets`, `tysel.sqlite`, `tysel.postgres`, `tysel.fs`, `tysel.llm` | [Host capabilities](runtime/capabilities.md) |
| `DurableContext`, `tysel.durable` | [Durable API](runtime/durable.md) |
| `test`, `assert`, `invokeFetch` | [Testing API](runtime/testing.md) |
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
