# Benchmarks and release evidence

## `tysel bench`

Run a benchmark suite and optionally record evidence.

```text
tysel bench <SUITE> [OPTIONS]
```

`SUITE` is one of `startup`, `memory`, `isolate`, `task`, `durable`, `http`,
or `all`.

| Option | Default | Meaning |
| --- | --- | --- |
| `--format <human|json>` | `human` | Select report format. |
| `--evidence <path>` | — | Write the versioned evidence document. |
| `--source-commit <sha>` | Discovered where possible | Record source identity. |
| `--command <text>` | Generated invocation | Record the reproducing command. |
| `--allow-unavailable` | — | Deprecated compatibility option. |

Complete release evidence requires a release-mode, full-scale `all` run.
Optional measurements can be marked `skipped` when their external dependency
is unavailable; only documented admission metrics can fail the gate.

## `tysel release`

Sign artifacts and verify release or reproducibility evidence.

| Subcommand | Required arguments | Important options |
| --- | --- | --- |
| `sign` | `<ARTIFACT> --key <key>` | Sign an evidence artifact. |
| `verify` | `<ARTIFACT> --trust <key>` | Verify an evidence signature. |
| `key-info` | `--key <key>` | Print public key information. |
| `reproduce` | `<FIRST> <SECOND> --source-commit <sha> --target <triple> --toolchain <id> --command <cmd>... --output <file>` | `--lockfile` defaults to `Cargo.lock`. |
| `verify-reproducibility` | `<ARTIFACT> --evidence <file> --target <triple>` | `--lockfile` defaults to `Cargo.lock`. |
| `sign-artifact` | `<ARTIFACT> --target <triple> --key <key>` | Bind an artifact to target metadata. |
| `verify-artifact` | `<ARTIFACT> --trust <key> --target <triple>` | Verify artifact and target binding. |

Private signing material must not be stored in the repository or passed
through logs. Use subcommand `--help` for the exact installed evidence format.

See [Performance and evidence](../../performance/README.md) for measurement
methodology and admission policy.
