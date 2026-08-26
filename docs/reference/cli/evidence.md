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

Sign and verify application release evidence.

| Subcommand | Required arguments | Important options |
| --- | --- | --- |
| `sign` | `<ARTIFACT> --key <key>` | Sign an evidence artifact. |
| `verify` | `<ARTIFACT> --trust <key>` | Verify an evidence signature. |
| `key-info` | `--key <key>` | Print public key information. |

`sign` and `verify` take an application executable, validate its complete
release sidecar set, and create or check `<artifact>.evidence.sig.json`.

Private signing material must not be stored in the repository or passed
through logs. Use subcommand `--help` for the exact installed evidence format.

See [Reproducible application releases](../../guides/reproducible-release.md)
for the complete artifact flow and [Performance and evidence](../../performance/README.md)
for measurement methodology and admission policy.
