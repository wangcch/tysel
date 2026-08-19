# Benchmarks

M0 measures a packaged `hello-service` against roadmap §30.

```bash
cargo build -p tysel-runtime --bin tysel-service --release
cargo run -p tysel-testkit --bin tysel-bench --release
```

| Directory | Metric |
|-----------|--------|
| `startup/` | Process spawn → `tysel listen` (p50 of 11 runs after 2 warmups) |
| `memory/` | Idle RSS (macOS) or PSS (Linux) 400ms after listen |
| `http/` | Request-body limit is enforced in the runtime (413); throughput later |

Cold start must be ≤ 15ms, idle memory ≤ 32MB, packaged binary ≤ 20MB. Linux PSS is the memory gate of record and is enforced by the `§30 linux pss` GitHub Actions job; macOS reports RSS as a proxy and cannot claim the gate.
