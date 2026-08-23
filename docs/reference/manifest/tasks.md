# Manifest tasks

The optional `[tasks]` map defines project workflows as Tysel argument vectors,
without a platform shell or `package.json` scripts.

```toml
[tasks.verify]
description = "Check and test"
steps = [
  ["check"],
  ["test"],
]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
```

## Task object

| Field | Required | Contract |
| --- | --- | --- |
| `description` | No | Human-readable string or `null`. |
| `depends` | One of `depends` or `steps` | Unique task names executed before this task. |
| `steps` | One of `depends` or `steps` | Ordered, non-empty argv arrays. |

A task name begins with a letter or digit and then contains letters, digits,
`:`, `_`, or `-`. At least one of `depends` or `steps` must be non-empty.
Unknown dependencies, dependency cycles, duplicate dependencies, and empty
steps are rejected while loading the manifest.

## Step commands

The first argument of each step must be one of:

```text
check  test  build  inspect  compat  run  dev  mcp  queue  image
```

Steps cannot call `init`, `config`, `task`, `doctor`, `upgrade`, `bench`, or
`release`. They also reject `--`, `-C`, `--project`, `--project-dir`, and
`--manifest`, preventing a step from escaping the selected project.

The runner does not invoke a shell. Each string is one argument, so pipes,
redirection, expansion, and shell quoting do not apply.

## Execution

```sh
tysel task --list
tysel task verify
```

Dependencies execute once in dependency order. Steps execute in declaration
order and inherit the task's project root and manifest. The first failed step
stops the workflow and makes the command exit non-zero.

See [`tysel task`](../cli/tasks.md) for invocation and
[Projects and configuration](../../concepts/projects-and-configuration.md) for
workflow design guidance.
