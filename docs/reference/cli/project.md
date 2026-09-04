# Project and configuration commands

## `tysel init`

Create a project or safely add Tysel files to an existing directory.

```text
tysel init [PATH] [OPTIONS]
```

When `PATH` is omitted in an interactive terminal, `init` asks for a project
directory and suggests `my-tysel-app`. In non-interactive use it defaults to
`.`. With global `-C`, a relative path is resolved from the selected base
directory.

| Option | Accepted values | Behavior |
| --- | --- | --- |
| `--template` | `http`, `worker`, `mcp`, `minimal` | Select generated application content. |
| `--manifest-format` | `toml`, `json` | Select only the manifest serialization; both use the same schema and combine with every other option. |
| `--entry <path>` | Project-relative path | Reuse an existing JavaScript or TypeScript entry. A newly generated entry must use `.ts`, `.tsx`, or `.mts`; `.wasm` is rejected because `init` does not yet generate Component projects. |
| `--package-json` | `auto`, `create`, `reuse`, `none` | Control only the optional npm sidecar. `auto` reuses an existing file or creates one; `create` requires no existing file; `reuse` requires one; `none` leaves it absent or untouched. |
| `--add-scripts` | Flag | Add Tysel scripts to a reused `package.json`. |
| `--package-manager` | `npm`, `pnpm`, `yarn`, `bun` | Select the install command shown or run for a generated package. The nearest lockfile is used when omitted. |
| `--install` | Flag | Run the selected package manager after project files are created. Requires `init` to create `package.json`. |
| `--verify` | Flag | Run `tysel check` after creation and optional installation. A newly generated package requires `--install` so TypeScript is not skipped. |
| `--no-tests` | Flag | Do not create the starter test, test dependency/script, or `test` step in the generated `verify` task. |
| `--dry-run` | Flag | Print the plan without writing files. |
| `--json` | Flag | Serialize a dry-run plan, including before/after file contents, as JSON. Requires `--dry-run`. |
| `--diff` | Flag | Include full unified file diffs in human-readable dry-run output. Requires `--dry-run`. |
| `-y, --yes` | Flag | Accept the proposed plan. |
| `--no-interactive` | Flag | Disable prompts and use documented defaults for omitted choices. |

`init` does not overwrite existing files silently. Use `--dry-run` in existing
repositories and review the adoption plan before accepting it.
An existing `.gitignore` is extended only with missing Tysel-generated paths;
its other entries are preserved.

With no path, the first interactive choice explicitly separates creating a new
project from adding Tysel to the current directory. Other choices support the
arrow keys and Enter. Terminals without cursor control fall back to numbered
choices.
Project destinations and entry paths are validated at their prompts so invalid
values can be corrected before the rest of the wizard runs.

For adopted projects, the default plan reports the individual package scripts
and ignore patterns that will be added. `--diff` exposes exact content changes,
including formatting changes from rewriting `package.json`; `--json` provides
the same before/after contents for automation.

Template, manifest format, entry, package integration, and tests are
independent choices. A `package.json` is not required to run, test, or build an
application that has no npm dependencies. Fresh projects receive
`tsconfig.json`; projects that
already have JavaScript or TypeScript configuration receive a separate
`tsconfig.tysel.json` when needed.
If that Tysel-specific config already has an explicit `files` list, `init`
requires it to include the selected entry and any generated typed test.
Comments and trailing commas accepted by TypeScript configuration files are
supported during this validation.

For a Wasm Component, start from the published Rust or Go Component starter and
its version-matched manifest; follow the
[Rust](../../guides/wasm-component-rust.md) or
[Go](../../guides/wasm-component-go.md) guide instead of `tysel init`.

```sh
tysel init api --template http --manifest-format toml --yes
tysel init . --package-json reuse --add-scripts --dry-run
tysel init . --package-json reuse --add-scripts --dry-run --diff
tysel init worker --template worker --manifest-format json --package-json none --yes
tysel init tool --template mcp --package-json none --no-tests --yes
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
