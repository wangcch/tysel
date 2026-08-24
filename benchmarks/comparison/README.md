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
runtime order. One set of four seeds is one record cycle. Run three complete
cycles without changing the source commit, binaries, toolchains, machine, or
runner settings. Do not publish a winner from quick mode or from a dirty
workspace.

Aggregate exactly four clean, rotated evidence files into one cycle report:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-report -- \
  --input target/benchmark-comparison/comparison-v1-x86_64-cycle1-seed*.json \
  --output target/benchmark-comparison/summary-v1-x86_64-cycle1.json
```

After all three cycles, enforce the publication stability contract. CPU
efficiency has a 10% relative-spread limit; throughput and latency are 15%
guardrails. A failed check remains visible in JSON and blocks publication:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-stability -- \
  --input target/benchmark-comparison/summary-v1-x86_64-cycle{1,2,3}.json \
  --output target/benchmark-comparison/stability-v1-x86_64.json \
  --fail-on-unstable
```

The GitHub Actions record job checkpoints evidence outside the checkout under
`/home/actions/tysel-benchmark-records/<commit>/<architecture>` and uploads a
remote checkpoint after every completed cycle. Seed and summary files are
atomically replaced, and stale stability evidence is removed before a resume.
Set `record_arch`, `record_cycles`, and `record_seeds` when resuming. A
stability failure should normally rerun all four seeds of the outlying cycle;
rerun a single seed only when the original execution itself failed before
producing valid evidence. Matrix fail-fast is disabled so a failure on one
architecture does not cancel the other.

For internal regression analysis, compare the new summary with a prior summary
from the same fixed host. The gate defaults to Tysel metrics only; peer changes
remain visible as environmental controls:

```bash
cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-report -- \
  --input target/benchmark-comparison/comparison-v1-x86_64-cycle1-seed*.json \
  --output target/benchmark-comparison/summary-v1-x86_64-cycle1.json \
  --baseline baselines/summary-v1-x86_64.json \
  --regression-threshold-pct 5 \
  --fail-on-regression \
  --gate-runtime tysel \
  --gate-metric requests-per-server-cpu-second-p50
```

The regression gate defaults to Tysel CPU efficiency only. Use
`--gate-metric all` only when intentionally promoting every supporting metric
to a release gate.

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

## Sustained-load diagnosis

The formal comparison is deliberately probe-free. Diagnose time-dependent
large-response degradation in a separate run on the same class of dedicated
Linux host:

```bash
benchmarks/comparison/profile-sustained-linux.sh \
  --response bytes-64k --path /bytes/64k \
  --concurrency 100 --duration-seconds 120 \
  --output target/benchmark-comparison/profile-bytes64k \
  --fail-on-degradation --max-degradation-pct 5
```

This builds a symbolized release-equivalent `profiling` Tysel binary and records
one-second load windows, load-generator CPU, per-core frequency, per-thread CPU
ticks, and a `perf` call graph when permitted. `window-analysis.json` compares
the first and last thirds of the run. `thread-cpu-summary.csv` identifies which
Tysel threads consumed CPU; `perf-report.txt` is the QuickJS/native hotspot
evidence. Use `--require-perf` when a missing call graph must fail the run.
`load.json.startedAtUnixMs` aligns each load window with the absolute timestamps
in `cpu-frequency.csv` and `thread-cpu-windows.csv`; thread CPU samples also
record the processor number so worker utilization can be matched to that core's
frequency. `frequency-phase-summary.csv` and `thread-phase-summary.csv`
automatically compare the first and last thirds of the same load interval.

The `diagnose` workflow input runs both 64 KiB workloads on the dedicated arm64
and x86_64 runners, requires usable frequency/thread/perf evidence, preserves
artifacts even when the contract fails, and accepts at most a 5% first-to-last
throughput decline.

Do not copy numbers from this diagnostic run into the cross-runtime report. Its
purpose is to establish a cause and verify that large-response throughput no
longer falls with elapsed time before starting new record cycles.

## External load-host verification

Only after sustained stability passes, copy `target/release/tysel` from the
record host to the dedicated server host and verify its SHA-256 against the
Tysel `executableSha256` retained in the cycle summaries. Start that exact
binary with the externally bound but otherwise identical manifest:

```bash
sha256sum ./tysel
./tysel run --manifest benchmarks/comparison/adapters/tysel/tysel-external.toml
```

On a separate load host, build or copy `tysel-bench-load` and run the same path,
response, concurrency, duration, and one-second windows against the server IP:

```bash
./tysel-bench-load \
  --address SERVER_IP:39001 \
  --path /bytes/64k --response bytes-64k \
  --concurrency 100 --duration-seconds 120 \
  --output external-bytes64k.json
```

Accept the replication only when the server binary hash matches the cycle
summary, there are zero errors, `sustainedChangePct` is at least -5%, and
`clientCpuCorePct / (logicalCpus * 100)` stays below 75%. Otherwise the external
result is client-capacity evidence, not a server ranking.

HTTP/2, streaming, SSE, WebSocket, build cost, load PSS/CPU, standalone binaries,
and isolation modes are follow-up tracks. Tysel task and durable metrics are not
placed in the common runtime ranking unless a semantically equivalent peer
implementation is defined.

## Publication policy

Use the report internally first. A result becomes eligible for the website only
after three stable four-seed record cycles on dedicated Linux x86_64 and arm64
hosts, followed by same-binary external load-host replication. Treat differences
inside ±5% as practically equivalent, publish the raw evidence, and show missing
or unstable cases explicitly instead of converting them to zero. CPU efficiency
is the primary multi-worker claim; total C100 throughput alone is insufficient.
