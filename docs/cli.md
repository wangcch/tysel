# CLI reference

Run `tysel <command> --help` for the complete arguments accepted by the
installed version.

## Development

Project commands discover the nearest `tysel.toml` or `tysel.json` from the
current directory upward. Use `-C/--project` to run as if invoked from
another project directory without changing the caller's shell, or
`--manifest` to select one file exactly:

```sh
tysel -C examples/sqlite-worker check
tysel run --manifest examples/sqlite-worker/tysel.toml
```

`-C` and `--manifest` are mutually exclusive. If both manifest formats exist
in one directory, implicit discovery fails instead of choosing one.

Project commands switch to the discovered project root before reading entries
or runtime-relative paths. For `init`, `-C` acts as the base directory for a
relative path:

```sh
tysel -C services init api --template http --yes
```

| Command | Purpose | Common options |
| --- | --- | --- |
| `tysel init [path]` | Create or safely add Tysel to a project. | `--template`, `--manifest-format`, `--entry`, `--package-json`, `--add-scripts`, `--no-tests`, `--dry-run`, `--yes`, `--no-interactive` |
| `tysel config ...` | Locate, validate, convert, and inspect configuration. | `path`, `validate`, `show`, `convert`, `schema` |
| `tysel task [name]` | List or run manifest-native task workflows. | `--list`, `--manifest` |
| `tysel check` | Validate manifest, bundle, TypeScript, capabilities, and Node builtin usage. | `--manifest` |
| `tysel compat` | Classify dependencies and imports. | `--json`, `--strict`, `--deny-unknown` |
| `tysel test [paths...]` | Run application tests in fresh QuickJS isolates. | `--timeout-ms`, `--json`, `--manifest` |
| `tysel dev [entry]` | Watch, bundle, and serve with reload. | `--manifest` |
| `tysel run [entry]` | Bundle and serve without watching files. | `--manifest` |
| `tysel inspect` | Print effective capabilities and limits. | `--manifest` |

## Tasks and protocols

Project workflows can be declared without `package.json` shell scripts:

```toml
[tasks.verify]
steps = [["check"], ["test"]]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
```

Run them with `tysel task verify` or list them with `tysel task --list`.
Each step is a Tysel argument vector, not a shell command, so quoting and
execution are consistent across supported platforms. Dependencies execute once
in dependency order, a failed step stops the workflow, and every step inherits
the task's project root and manifest.

| Command | Purpose |
| --- | --- |
| `tysel queue <name> --input <json>` | Submit one message to a registered Queue handler and print its result. |
| `tysel mcp` | Serve registered MCP tools over bounded newline-delimited stdio. |

Both commands accept an optional entry and `--manifest`.

## Configuration tools

```sh
tysel config path
tysel config validate
tysel config show --format json
tysel config convert --to json
mkdir -p generated
tysel config convert --to json --output generated/tysel.json
tysel config schema
```

`convert` writes to stdout unless `--output` is explicit. Output files use
create-new semantics and are never overwritten. The output directory must
already exist, and the filename extension must match `--to`.

`config path` only performs discovery, so it can still locate a malformed
manifest for troubleshooting. Other config commands load and validate it.
`config show` expands omitted defaults. `config schema` does not require a
project and prints the bundled Draft 2020-12 JSON Schema.

See [Projects and configuration](concepts/projects-and-configuration.md) for
init adoption behavior, path resolution, and task restrictions.

## Packaging and release

| Command | Purpose | Common options |
| --- | --- | --- |
| `tysel build` | Emit one native application executable. | `--release`, `--stub`, `--output`, `--profile`, `--target`, `--manifest` |
| `tysel image` | Generate a non-root Linux container context and optionally call Docker. | `--binary`, `--tag`, `--base-image`, `--context-only`, `--force` |
| `tysel release ...` | Sign, verify, and inspect release evidence. | Use subcommand help. |
| `tysel bench <suite>` | Run `startup`, `memory`, `isolate`, `task`, `durable`, `http`, or `all`. | `--format human|json`, `--evidence` |

`tysel build --target` validates a target name but currently accepts only the
host platform. `--release` also writes checksum, compatibility, SBOM, license,
and evidence sidecars.

Complete benchmark evidence requires a release-mode, full-scale `all` run.
Benchmark JSON schema v2 includes raw samples and available percentiles. Only
documented release-admission metrics can fail their gate; optional Postgres
measurements are marked `skipped` without a configured test URL.

## Installation health

| Command | Purpose | Common options |
| --- | --- | --- |
| `tysel doctor` | Diagnose installation, platform, and project state. | `--install`, `--project`, `--network`, `--json` |
| `tysel upgrade` | Check, atomically upgrade, or roll back a managed installation. | `--check`, `--version`, `--channel`, `--yes`, `--force`, `--rollback`, `--json` |

Network diagnostics are opt-in with `doctor --network`.
For backward compatibility, `doctor --project` accepts either a project
directory or an explicit `tysel.toml`/`tysel.json`; other commands use
`--manifest` when selecting a file.

## Output and exit behavior

Successful commands exit with status 0. Validation, policy, test, build,
benchmark-gate, and runtime failures exit non-zero.

Use the global option before the command:

```sh
tysel --error-format json check
```

Fatal errors are emitted on stderr as an object containing `code` and
`message`. Machine-readable command reports, including `test --json`,
`compat --json`, and `bench --format json`, are emitted on stdout.
