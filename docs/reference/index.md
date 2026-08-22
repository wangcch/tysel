# Reference

Use these pages to look up exact commands, configuration, APIs, support, and
operational limits. For a complete workflow, start with [Guides](../guides/index.md).

## Product reference

| Surface | Reference | Source of truth |
| --- | --- | --- |
| Commands, options, output, and exit behavior | [CLI reference](../cli.md) | Native CLI command definitions |
| TOML and JSON application configuration | [Manifest reference](manifest.md) | Bundled Draft 2020-12 JSON Schema |
| Application exports and host APIs | [Runtime API](../api/runtime.md) | Runtime implementation and `@tysel/types` |
| Web and JavaScript APIs | [JavaScript compatibility](../architecture/javascript-runtime-compatibility.md) | Runtime compatibility inventory |
| Grants by execution profile | [Capability matrix](../capabilities/README.md) | Manifest, profile, and host enforcement |
| npm and Node.js assumptions | [npm compatibility](../compatibility/README.md) | Compatibility catalog plus project scan |

## Operational reference

| Need | Reference |
| --- | --- |
| Install, diagnose, upgrade, or roll back | [Installation](../install.md) |
| Understand trust boundaries and deployment responsibilities | [Security model](../security/README.md) |
| Deploy, back up, restore, monitor, or respond to incidents | [Production operations](../operations/production.md) |
| Reproduce and interpret benchmark evidence | [Performance and evidence](../performance/README.md) |

## Quick lookup

```sh
tysel --help
tysel <command> --help
tysel config schema
tysel config show --format json
tysel inspect
tysel compat --json
```

The installed CLI and bundled schema are authoritative for that binary. The
website must not imply a stable public release until a matching tagged release
exists.
