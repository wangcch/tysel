# Projects and configuration

A Tysel project is a directory containing exactly one application manifest:
`tysel.toml` or `tysel.json`. Both formats use the same versioned schema. A
neighboring `package.json` is optional and never overrides Tysel configuration.

## Project discovery

Project-aware commands search for the nearest manifest from the current
directory upward. This lets commands run from a nested source directory:

```sh
cd my-service/src/routes
tysel check
```

Select another project without changing the shell directory with the global
`-C/--project` option:

```sh
tysel -C examples/hello-service check
tysel -C examples/hello-service build --release
```

Automation that needs one exact file can use `--manifest`:

```sh
tysel check --manifest services/api/tysel.json
```

`-C` and `--manifest` are mutually exclusive. Discovery fails when both
`tysel.toml` and `tysel.json` exist in the same directory; Tysel never chooses
one implicitly.

The discovered manifest directory becomes the project root. CLI project
commands resolve the application entry, SQLite storage, and filesystem roots
from it. A packaged executable has no external project manifest, so its
relative runtime paths resolve from the process working directory.

## Create or adopt a project

In a terminal, `tysel init` offers a recommended quick start and an optional
customized flow:

```sh
tysel init my-service
```

The customized flow selects:

- HTTP service, Queue worker, MCP tool, or minimal template;
- TOML or JSON manifest;
- generated or existing application entry;
- package creation, reuse, or no `package.json`;
- whether to generate tests.

Every choice is also available as an option for automation:

```sh
tysel init worker \
  --template worker \
  --manifest-format json \
  --package-json none \
  --no-tests \
  --yes
```

Use `--dry-run` to print the complete change plan without writing. `init`
checks every destination first, preserves existing source and configuration,
rejects symlinked mutation targets, and rolls back its writes if a later
operation fails.

When adopting an existing JavaScript project, Tysel defaults to
`src/tysel.ts` and `tsconfig.tysel.json`. It preserves `package.json` unless
`--add-scripts` explicitly requests namespaced `tysel:*` scripts:

```sh
tysel init . --dry-run
tysel init . --add-scripts
```

Use `--no-interactive` in CI to disable prompts while retaining documented
defaults. Use `--yes` when all supplied choices should be accepted without
confirmation.

## Inspect and convert configuration

The `config` commands use the same discovery and validation path as execution:

```sh
tysel config path
tysel config validate
tysel config show
tysel config show --format json
tysel config schema
```

`config path` reports the discovered file without parsing it, which is useful
when repairing an invalid manifest. `config validate` reports the format,
project root, optional package sidecar, and validation status. `config show`
prints the effective manifest with defaults expanded.

Convert the effective configuration between TOML and JSON:

```sh
tysel config convert --to json
mkdir -p generated
tysel config convert --to json --output generated/tysel.json
```

Conversion writes to stdout unless `--output` is present. It never overwrites a
file, never creates the parent directory implicitly, and requires the filename
extension to match the requested format. Tysel also refuses to create a second
root manifest that would make discovery ambiguous.

`tysel config schema` prints the bundled Draft 2020-12 JSON Schema for editor,
CI, and tooling integration. The runtime validator remains authoritative.

## Reproducible project tasks

The manifest can compose Tysel commands without shell-specific quoting or a
required `package.json`:

```toml
[tasks.verify]
description = "Check and test"
steps = [["check"], ["test"]]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
```

```sh
tysel task --list
tysel task verify
```

Dependencies execute once in dependency order. Steps run from the project root
using the same manifest. A failed step stops the task. To keep execution
bounded and reproducible, steps cannot invoke another task or override project
selection, and only the commands listed in the
[manifest reference](../reference/manifest.md#tasks) are accepted.

Continue with the [getting started guide](../getting-started.md),
[CLI reference](../cli.md), or [manifest reference](../reference/manifest.md).
