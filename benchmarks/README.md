# Benchmarks

M0 measures a packaged `hello-service` against roadmap §30.

```bash
cargo build -p tysel-runtime --bin tysel-service --release
cargo run -p tysel-cli -- bench all --allow-unavailable
cargo run -p tysel-cli -- bench all --format json --allow-unavailable
```

`tysel bench startup` and `tysel bench memory` run the existing cold-start and
idle-memory harness. `isolate`, `task`, and `durable` report `unavailable` until
those suites exist; they never emit placeholder numbers. `all` returns non-zero
while any suite is unavailable unless `--allow-unavailable` is explicit.
`--allow-unavailable` cannot be combined with `--evidence`, so an incomplete
matrix cannot produce release evidence.

The release job still records evidence with:

```bash
cargo run -p tysel-testkit --bin tysel-bench --release
```

CI also writes a strict machine-readable evidence document containing the raw
samples, gate decisions, artifact digest, source commit, command, CPU, OS, and
target. To produce the same document locally:

```bash
cargo run -p tysel-testkit --bin tysel-bench --release -- \
  --evidence target/benchmark-evidence.json \
  --source-commit 0123456789abcdef0123456789abcdef01234567 \
  --command "cargo run -p tysel-testkit --bin tysel-bench --release"
```

| Directory | Metric |
|-----------|--------|
| `startup/` | Process spawn → `tysel listen` (p50 of 11 runs after 2 warmups) |
| `memory/` | Idle RSS (macOS) or PSS (Linux) 400ms after listen |
| `http/` | Request-body limit is enforced in the runtime (413); throughput later |

Cold start must be ≤ 15ms, idle memory ≤ 32MB, packaged binary ≤ 20MB. Linux PSS is the memory gate of record and is enforced by the `linux-pss` GitHub Actions job; macOS reports RSS as a proxy and cannot claim the gate.
