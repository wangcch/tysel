# CLI reference

| Command | Purpose | Important options |
| --- | --- | --- |
| `tysel init [path]` | Create a non-destructive service and test skeleton. | Fails before writing on conflicts. |
| `tysel check` | Validate manifest, bundle, TypeScript, and Node builtin usage. | `--manifest` |
| `tysel compat` | Classify dependencies and entry imports. | `--json`, `--strict`, `--deny-unknown` |
| `tysel test [paths…]` | Run `.test.*` files in isolated QuickJS instances. | `--timeout-ms`, `--json` |
| `tysel dev` / `run` | Serve with or without source reload. | `--manifest` |
| `tysel queue name` / `mcp` | Invoke Queue or serve MCP ingress. | See command help. |
| `tysel inspect` | Print effective capabilities and limits. | `--manifest` |
| `tysel build` | Emit one native executable. | `--release`, `--stub`, `--output`, `--target` |
| `tysel image` | Generate a non-root container context and optionally call Docker. | `--binary`, `--tag`, `--base-image`, `--context-only`, `--force` |
| `tysel bench <suite>` | Run the §30 harness (`startup`, `memory`, `all`; others report `unavailable`). `all` is strict unless unavailable suites are explicitly allowed. | `--format human\|json`, `--allow-unavailable` (with `all` only), `--evidence` (requires every suite) |
| `tysel release …` | Sign and verify release evidence. | See command help. |

Successful commands exit 0. Validation, policy, test, build, benchmark, and runtime failures exit non-zero. Global `--error-format human|json` controls fatal errors on stderr; command reports such as `test --json`, `compat --json`, and `bench --format json` go to stdout.
