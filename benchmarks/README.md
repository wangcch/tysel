# Benchmarks

The harness measures the implemented roadmap §23 matrix and keeps release gates separate
from observational measurements.

```bash
cargo build -p tysel-runtime --bin tysel-service --release
cargo build -p tysel-isolate --bin tysel-worker --release
cargo run -p tysel-cli --release -- bench all
cargo run -p tysel-cli --release -- bench all --format json
```

Set `TYSEL_BENCH_QUICK=1` for a reduced local smoke run. Quick mode is useful for
correctness checks, but it is not release evidence. Full multi-suite mode uses 101
samples so p99 is meaningful; the legacy cold-start gate retains its established
11-sample method. `TYSEL_DURABLE_POSTGRES_URL` enables the optional
Postgres row; without it that metric is explicitly marked `skipped`.

The legacy release job records the three original admission gates with:

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

To record the multi-suite schema v2 document, run the release CLI from a clean
workspace (or pass an explicit source commit):

```bash
cargo run -p tysel-cli --release -- bench all \
  --format json \
  --evidence target/benchmark-evidence-v2.json
```

| Directory | Metric |
|-----------|--------|
| `startup/` | Process spawn → `tysel listen` (p50 of 11 runs after 2 warmups) |
| `memory/` | Idle RSS (macOS) or PSS (Linux) 400ms after listen |
| `isolate/` | Cold/warm creation, dispatch reuse, idle memory, timeout and crash replacement |
| `task/` | Queue scaling, claim/commit, cancellation/deadline transitions, leases and backpressure |
| `durable/` | SQLite/Postgres append, suspend/resume, replay, signals and restart recovery |
| `http/` | HTTP/1.1 keep-alive, HTTP/2, JSON sizes, streaming, WebSocket, SSE and protocol-specific concurrency |

Cold start must be ≤15ms, idle memory ≤32MB, packaged binary ≤20MB, warm isolate
creation ≤5ms, and Durable Task resume ≤10ms. Other values are observations until
the roadmap defines a reproducible baseline subtraction or threshold. Linux PSS is
the memory gate of record; macOS RSS is a proxy.
