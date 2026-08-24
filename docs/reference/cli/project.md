# Project and configuration commands

## `tysel init`

Create a project or safely add Tysel files to an existing directory.

```text
tysel init [PATH] [OPTIONS]
```

`PATH` defaults to `.`. With global `-C`, a relative path is resolved from the
selected base directory.

| Option | Accepted values | Behavior |
| --- | --- | --- |
| `--template` | `http`, `worker`, `mcp`, `minimal` | Select generated application content. |
| `--manifest-format` | `toml`, `json` | Select the manifest format. |
| `--entry <path>` | Project-relative non-Wasm path | Override the generated JavaScript application entry. `.wasm` is rejected because `init` does not yet generate Component projects. |
| `--package-json` | `auto`, `create`, `reuse`, `none` | Control npm sidecar adoption. |
| `--add-scripts` | Flag | Add Tysel scripts to a reused `package.json`. |
| `--no-tests` | Flag | Do not create the starter test. |
| `--dry-run` | Flag | Print the plan without writing files. |
| `-y, --yes` | Flag | Accept the proposed plan. |
| `--no-interactive` | Flag | Fail instead of prompting when input is required. |

`init` does not overwrite existing files silently. Use `--dry-run` in existing
repositories and review the adoption plan before accepting it.

For a Wasm Component, start from the repository's Rust or Go fixture and its
checked-in manifest; follow the
[Rust](../../guides/wasm-component-rust.md) or
[Go](../../guides/wasm-component-go.md) guide instead of `tysel init`.

```sh
tysel init api --template http --manifest-format toml --yes
tysel init . --package-json reuse --add-scripts --dry-run
```

## `tysel config`

Locate, validate, inspect, convert, or print the schema for a manifest.

| Subcommand | Syntax | Output and behavior |
| --- | --- | --- |
| `path` | `tysel config path [--manifest FILE]` | Print the discovered path. It can locate a malformed manifest. |
| `validate` | `tysel config validate [--manifest FILE]` | Load and validate configuration. |
| `show` | `tysel config show [--format toml|json] [--manifest FILE]` | Print effective configuration with omitted defaults expanded. |
| `convert` | `tysel config convert --to toml|json [-o FILE] [--manifest FILE]` | Convert a validated manifest. Defaults to stdout. |
| `schema` | `tysel config schema` | Print the bundled Draft 2020-12 JSON Schema; no project is required. |

`config convert --output` uses create-new semantics: it never overwrites a
file, the parent directory must already exist, and the filename extension must
match `--to`.

```sh
tysel config validate
tysel config show --format json
tysel config convert --to json --output generated/tysel.json
tysel config schema
```

See [Manifest reference](../manifest/index.md) for every field and
[Projects and configuration](../../concepts/projects-and-configuration.md) for
discovery and adoption workflows.
