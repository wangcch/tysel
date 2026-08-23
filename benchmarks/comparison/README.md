# Cross-runtime comparison

This harness compares Tysel, Node.js, Bun, and Deno with matched external
HTTP workloads. It is separate from `tysel bench all`: Tysel release gates stay
stable, while peer results remain observational.

The primary track is `runtime-source` for every runtime. Standalone executables
will be a separate track; results from the two modes must not be mixed.

All four adapters are strict-checked with the repository-locked TypeScript
7.0.2 compiler before runtime measurement. Node 24, Bun, Deno, and Tysel then
execute their matched `.ts` entry points through their native TypeScript paths.
Type-check and build time are deliberately outside startup and HTTP timing. The
evidence records the expected and actual TypeScript versions plus the compiler
launcher hash, while the runner snapshot also records the `pnpm-lock.yaml` hash.

All adapters disable per-request access logging. Logging changes throughput,
latency, and memory enough that enabling it for only one runtime would invalidate
the comparison.

## Record targets

- Linux x86_64 on a fixed `tysel-benchmark` runner
- Linux arm64 on a fixed `tysel-benchmark` runner

Never aggregate rankings across architectures. A report is valid only for the
architecture, CPU, kernel, runtime lock, source commit, and command recorded in
its evidence file.

## Run

The runtime versions in `runtimes.lock.json` are exact. Provision those
versions before producing record evidence. Quick mode is only a correctness
smoke and may explicitly retain unavailable runtimes. Adapter failures and
response-contract violations still fail the run:

On Linux, provision the checksum-locked Node, Bun, and Deno archives inside
`target/benchmark-comparison/tools` and prepend the printed directory to `PATH`:

```bash
tool_bin="$(benchmarks/comparison/provision-linux.sh)"
export PATH="${tool_bin}:${PATH}"
npm exec --yes --package=pnpm@11.5.0 -- pnpm install --frozen-lockfile
benchmarks/comparison/runner-doctor-linux.sh --strict
```

The provisioner supports x86_64 and aarch64, verifies the official SHA-256 for
every archive, and never installs globally. The strict doctor records the CPU,
kernel, memory, load, governor/turbo state, Git state, runtime paths, versions,
and executable hashes. It rejects dirty workspaces, unexpected runtime versions,
non-performance CPU governors, and heavily loaded runners.

```bash
cargo run --locked -p tysel-bench-compare --bin tysel-bench-compare -- \
  --quick --allow-missing \
  --output target/benchmark-comparison/smoke.json
```

Full internal evidence fails closed when any runtime is absent or has the wrong
version:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-compare -- \
  --output target/benchmark-comparison/comparison-v1.json \
  --order-seed 1
```

Repeat the full run with order seeds `1`, `2`, `3`, and `4` to rotate the
runtime order. Do not publish a winner from quick mode or from a dirty workspace.

Aggregate exactly four clean, rotated evidence files into the architecture-level
technical report:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-report -- \
  --input target/benchmark-comparison/comparison-v1-x86_64-seed*.json \
  --output target/benchmark-comparison/summary-v1-x86_64.json
```

For internal regression analysis, compare the new summary with a prior summary
from the same fixed host. The gate defaults to Tysel metrics only; peer changes
remain visible as environmental controls:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-report -- \
  --input target/benchmark-comparison/comparison-v1-x86_64-seed*.json \
  --output target/benchmark-comparison/summary-v1-x86_64.json \
  --baseline baselines/summary-v1-x86_64.json \
  --regression-threshold-pct 5 \
  --fail-on-regression \
  --gate-runtime tysel
```

Aggregation fails if commits, architecture/CPU/kernel fingerprints, source
toolchains, matrices, runtime versions, executable hashes, workload sets, or
memory kinds differ. It also rejects duplicate execution orders, unavailable
runtimes, dirty evidence, and any recorded HTTP error. Each summary retains the
path, run ID, and SHA-256 of every source evidence file.

## Current v1 scope

The first executable slice covers readiness startup, idle process-tree memory,
HTTP/1.1 keep-alive, 1 KiB/64 KiB JSON, a 64 KiB byte response, and concurrency
1/10/100. Linux rounds also record server and load-generator CPU in core-percent,
peak process-tree memory, and requests per server CPU-second. The latter makes CPU
efficiency comparable even when runtimes use different numbers of cores; client
CPU exposes likely load-generator saturation. The evidence keeps raw startup,
latency, resource, and per-round throughput samples, plus p50/p95/p99 where the
sample size supports them. Summary latency percentiles retain every request;
their deterministic median interval bootstraps round-level medians so correlated
requests inside one measurement round are not counted as independent trials.

HTTP/2, streaming, SSE, WebSocket, build cost, load PSS/CPU, standalone binaries,
and isolation modes are follow-up tracks. Tysel task and durable metrics are not
placed in the common runtime ranking unless a semantically equivalent peer
implementation is defined.

## Publication policy

Use the report internally first. A result becomes eligible for the website only
after three stable record cycles on both architectures. Treat differences inside
±5% as practically equivalent, publish the raw evidence, and show missing or
unstable cases explicitly instead of converting them to zero.
