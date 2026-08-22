# CLI reference

Install the three binaries first ([Install](install.md)). After `tysel` is on `PATH`, these commands are the product interface.

| Command | Purpose | Important options |
| --- | --- | --- |
| `tysel init [path]` | Create a non-destructive service and test skeleton. | Fails before writing on conflicts. |
| `tysel check` | Validate manifest, bundle, TypeScript, and Node builtin usage. | `--manifest` |
| `tysel compat` | Classify dependencies and entry imports. | `--json`, `--strict`, `--deny-unknown` |
| `tysel test [paths…]` | Run `.test.*` files in isolated QuickJS instances. | `--timeout-ms`, `--json` |
| `tysel dev` / `run` | Serve with or without source reload. | `--manifest` |
| `tysel queue name` / `mcp` | Invoke Queue or serve MCP ingress. | See command help. |
| `tysel inspect` | Print effective capabilities and limits. | `--manifest` |
| `tysel doctor` | Diagnose managed installation, platform, project, and opt-in release network access. | `--install`, `--project`, `--network`, `--json` |
| `tysel upgrade` | Check, atomically upgrade, or roll back a managed toolchain. | `--check`, `--version`, `--yes`, `--force`, `--rollback`, `--json` |
| `tysel build` | Emit one native executable. | `--release`, `--stub`, `--output`, `--target` |
| `tysel image` | Generate a non-root container context and optionally call Docker. | `--binary`, `--tag`, `--base-image`, `--context-only`, `--force` |
| `tysel bench <suite>` | Run the roadmap §23 harness (`startup`, `memory`, `isolate`, `task`, `durable`, `http`, or `all`). | `--format human\|json`, `--evidence` (requires release `all` at full scale) |
| `tysel release …` | Sign and verify release evidence. | See command help. |

Successful commands exit 0. Validation, policy, test, build, benchmark-gate, and runtime failures exit non-zero. Benchmark JSON schema v2 records every raw sample; full multi-suite runs use 101 samples and publish p50/p95/p99, while shorter or singleton measurements omit unsupported tail percentiles. Only metrics with an explicit roadmap gate can fail the command. Optional Postgres measurements are marked `skipped` when no test URL is configured. Global `--error-format human|json` controls fatal errors on stderr; command reports such as `test --json`, `compat --json`, and `bench --format json` go to stdout.
