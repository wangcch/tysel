# Agent triage cost experiment (P5)

## Evidence layout

Offline CI uses only the versioned [fixtures](fixtures/README.md). The
[named-volume protocol](protocol-volume.json) and [storage plan](storage-plan.json)
are byte-identical copies of the historical inputs. The frozen dispatcher plan
keeps its original provenance fields; `dispatch-ab.py` loads the stable local
volume protocol and verifies the plan's original SHA-256.

See the [result and archive index](../../docs/agent-execution-evidence/README.md)
for historical decisions, tradeoffs and known failures. Complete dated logs are
kept in the ignored `local-artifacts/` archive, outside Git; they are not needed
for CI. New measurements belong under `target/` or `local-artifacts/`, with a
fresh directory for each attempt. Preserve failed attempts with their run.
Historical paired commands below require the exact archived packages and their
recorded paths; a clean checkout contains the offline fixtures, not those packages.

The separate `caller-memory.py` attribution runner uses fresh namespaces and
the existing bounded 512-job workload to distinguish per-process PSS, private
and anonymous memory, thread counts, active outbox polling and fully quiet idle.
It keeps the caller alive after stopping only its own drained supervisor and
samples quiet memory at 5/30/60 seconds. It rejects missing processes, PID reuse
and sampler errors. Run it only on Linux with local test artifacts and a new
output directory, for example:

```sh
python3 benchmarks/agent-triage/caller-memory.py \
  --bin-dir /tools/release --support /tools/release/tysel-bench-agent-support \
  --output /volume/new-caller-diagnostic --kind baseline
```

Optional `--release` reuses exact packaged bytes, `--clients 8` selects the
eight-client/zero-adapter-delay cell, and `--rounds` controls repetitions. The
default covers four clients/20 ms and eight clients/0 ms, three rounds each.
`--kind instrumented` labels an externally built diagnostic artifact; it does
not add instrumentation to production. These observations do not replace the
full cost matrix or establish that a lower PSS sample proves a leak was fixed.
Offline sampler checks: `python3 benchmarks/agent-triage/test_caller_memory.py`.

Read [protocol.json](protocol.json) before running. Its thresholds are provisional
engineering budgets for an interactive, read-only internal ticket queue: a warm
job should finish within one second, sustain two jobs/s sequentially or six at
four clients, use at most 512 MiB PSS, and recover within the default 45-second
lease plus five seconds. Pure transformations have a 50 ms p95 budget. These are
local acceptance assumptions, not user-approved production SLOs. Never change
thresholds after seeing a result; a miss remains a miss in the evidence.

All three variants use the same deterministic triage function and return the same
summary. `direct` embeds it in a trusted service. `snapshot` embeds it in an
isolated worker with no ambient grants. Both receive already projected detail
and make one HTTP request, with zero adapter calls. `lookup` is the exact packaged
P4 caller/plugin flow: select, one authorized adapter read, summarize, Durable
history, application journal, audit and Python outbox driver. This comparison
measures the cost of the complete deployment choices; it cannot attribute their
difference solely to process isolation. It does not remove persistence to make
the lookup path faster.

Use ASCII subjects of 64 or 500 bytes and one or sixteen tickets. Record actual
serialized request/response sizes. Healthy runs use no retries. Adapter delays
of 0 and 20 ms apply only to `lookup`; record the actual sleep separately and
subtract it per job before aggregating the remaining elapsed time. This residual
still includes adapter HTTP, validation, storage, queueing, polling and fixture
overhead. It is not a measurement of CPU time. Unique ticket IDs per in-flight
client correlate independent adapter observations to the admitted jobs.

Cold means fresh native processes/worker and a fresh namespace, then the first
successful result, with warm OS filesystem caches. Compilation and packaging are
outside the timer. Lookup cold includes package verification and health check;
the other variants include executable readiness. Namespace initialization is
outside the timer for all variants. Twenty small-payload samples
per variant permit a descriptive p95. They do not measure machine boot or dropped
disk caches. Warm runs reuse processes/worker and execute eight warmups per cell,
then three rounds of 32 jobs at closed-loop concurrency one or four. Rotate variant
order by round. Sizes/concurrency/delay are distinct cells. Do not pool rounds to
claim independent confidence intervals. Report per-round throughput and latency
as well as pooled descriptive p50/p95. All clients use fresh loopback HTTP/1
connections; the full workflow retains its own transport settings and packaged
supervisor policy. P5–O3 use the 250 ms outbox poll; the O4 candidate below changes
completion follow-up only. Status observation polls every 5 ms, so end-to-end includes observation
and load-generator costs.

Lookup stage times come from the existing transactional audit: admission to first
reservation, reservation to accepted result for select/read/summarize, and last
acceptance to terminal. Audit timestamps have millisecond wall-clock resolution;
retain the raw events and reject negative intervals. They include authority and
storage work, not just remote handler time. Micro variants have one HTTP-transform
stage, equal to end-to-end. Recovery is three separately observed deployment crashes
after the independent adapter receives a read and before recording it; keep the
original 45-second lease and 120-second deadline. Report all samples and max,
not a tail estimate from three samples. Two physical reads per crash case are
required. Denial probes use the original isolated plugin's denied fetch and
filesystem routes and must not reach the adapter.

The existing `tysel-bench-compare` percentile/distribution and Linux per-process
PSS helpers are reused by `tysel-bench-agent-support`. A 50 ms sampler traverses
all process threads' children and sums caller, plugin and worker PSS; the driver,
fixture, sampler and load client share Python and are excluded. Retain per-sample
process counts and report sampled peak, not a guaranteed high-water mark. Include
idle measurements after warmup. This is a standalone-artifact internal experiment,
separate from the cross-runtime publication track and its dedicated-host gates.

The driver fails on transport, output, attempt-count, denial, cleanup or protocol
errors. Numerical budget misses are saved and return a distinct nonzero exit code.
Raw samples, frozen protocol hash, relevant source/binary hashes, command, OS,
artifact metadata and analysis must be retained alongside the ADR.

## Run on Linux

### O4.1 capacity diagnostic

`capacity.py` extends observation to 1/2/4/8 clients and namespaces with 0/900
completed jobs, without changing application limits or the historical P5/O4
protocols. The separate [capacity protocol](capacity-protocol.json) fixes three
rounds per cell, 64 measured jobs plus eight warmups, bounded payload and 0/20 ms
adapter delay. It is diagnostic evidence, not a replacement acceptance gate or
a production throughput ceiling.

```sh
python3 benchmarks/agent-triage/capacity.py \
  --bin-dir /path/to/current/linux/release \
  --output /volume/new-capacity-run
python3 benchmarks/agent-triage/test_capacity.py
```

Use fresh binaries built from the current source and a new output path on the
Podman named volume. The tool packages once, builds a 900-job checkpoint through
real authorized API calls, and restores the stopped checkpoint before each dense
round. Seed and measured work retain the original attempts, audit, ownership,
lease, deadline and retention rules. Final dense namespaces contain 972 jobs.
All application processes are stopped and each measured namespace is removed;
the source checkpoint/package and raw observations remain for investigation.

CPU uses changes in `utime` and `stime` for the native caller/plugin/worker
processes, with `SC_CLK_TCK` conversion. PID start times and process sets must
remain stable; a missing/replaced process invalidates the measurement. See the
[Linux proc field definitions](https://www.kernel.org/doc/html/latest/filesystems/proc.html).
These counters measure CPU, not database lock waiting or storage latency. The
counter-read window slightly brackets the 64-job throughput window. Python CPU
is recorded separately and combines the supervisor, fixture, observer, load
generator and sampling threads; it cannot be attributed to the supervisor alone.
PSS retains the original native-process scope and 50 ms sampling limitations.

HTTP observations cover startup, eight warmups, 64 measured jobs and shutdown;
per-job counts exclude the health route and divide by 72. They are not requests
per second for the shorter measured window. Reported HTTP durations include
queueing and transport and must not be summed as CPU time. Round order rotates,
but the cells are not matched A/B experiments. The tool refuses `python -O`
because reused real-workload checks contain assertions; the independent offline
accounting tests also run under optimized Python.

### Original cost matrix

Build matching release artifacts, then choose a new output directory:

```sh
cargo build --locked --release -p tysel-cli -p tysel-runtime -p tysel-isolate -p tysel-bench-compare --bins
PYTHONDONTWRITEBYTECODE=1 NO_PROXY=127.0.0.1,localhost \
  python3 benchmarks/agent-triage/run.py \
  --bin-dir target/release --output target/agent-cost-run
```

The output keeps `measurements.json`, the exact frozen protocol and all packaged
executables under `releases/`. Exit 0 means all checks pass, 2 means a numerical
budget miss, and exceptions attempt to save an error record before exiting
nonzero. If final persistence fails, retain stderr as well as the last complete
snapshot; that snapshot can still say `running` and is not complete evidence.
Keep failed runs too.
The driver deliberately has no quick mode that could accidentally be reported
as complete P5 evidence. A full run includes 60 cold samples, 1,536 measured warm
jobs across 48 rounds, 384 warmups, two denial probes and three recovery cases.
It takes several minutes because sequential lookup retains the outbox polling
and recovery retains the original lease. Runtime logging is unchanged from P4;
results include its cost. Do not apply the cross-runtime harness's logging-off
assumption to this deployment experiment.

Render the saved result independently of the runtime (also accepts `.json.gz`):

```sh
python3 benchmarks/agent-triage/report.py \
  --input target/agent-cost-run/measurements.json --output target/agent-cost-summary
```

The report independently checks the retained evidence before writing output.
It validates the frozen workload and budgets, full sample/round coverage,
recorded adapter/denial/cleanup contracts, and recomputes distributions,
throughput, sampled peak PSS and every budget check. Missing, duplicated,
non-finite or inconsistent data is rejected, including saved pass flags that
disagree with the samples. These guards also run under `python -O`.

Valid P5/P5.1/P5.2 archives reproduce the same report bytes and preserve their
14/15/0 misses. Exit 0 from the **report renderer** means the input is valid and
was rendered; a valid `budget_miss` remains a failed performance result. This
differs from the measurement driver's performance-sensitive exit codes.
Invalid input exits nonzero before creating or overwriting report files.
`results.md` is for reading; `summary.json` retains sizes, memory scope, stages
and counts without duplicating raw job samples.

This is internal-consistency verification, not a signature or proof that a
fully fabricated but consistent record came from a real run. Retain the raw
observations, protocol, artifact identities and environment alongside reports.
No application or sampling behavior changes as part of this offline check.

Run the archive replay and inconsistent-input regressions without native tools,
Podman or dependencies; the same command is wired into CI:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/agent-triage/test_report.py
```

The [improvement plan](../../docs/agent-execution-improvement-plan.md) separates
this report fix from deferred operations and performance experiments.

## Sampling evidence reliability (O3)

The sampling driver now handles interrupted evidence writes, unresponsive helpers
and teardown failures. A snapshot is fully JSON-encoded
before creating a unique temporary file in the destination directory. After
writing, flushing and closing the file, the driver replaces `measurements.json`.
Encoding, write or replacement failures retain the previous complete snapshot
and attempt to remove the temporary file. This avoids publishing partial JSON;
it does not add `fsync` or promise persistence through power loss. SIGKILL can
still leave a temporary file.

Helper calls share a five-second default deadline across lock acquisition,
writing and reading a complete response line. Closing waits up to five seconds
for exit after EOF, then up to one second each for terminate and kill. A timeout,
premature exit, invalid response or background sampling failure fails the run
rather than silently dropping samples. Every teardown is attempted even if
another teardown fails. The final record's `error` retains the primary measurement
failure and nested application, fixture and sampler cleanup exception chains;
top-level `cleanupErrors` records helper-close and final timing-metadata errors.
Final persistence is attempted after cleanup. If that save also fails, stderr
retains all failure causes through exception chains. A previous `running` snapshot
is useful for diagnosis but must not be reported as a completed measurement.

The original 23 fault-injection tests cover 28 branches and pass under normal and optimized
Python on both macOS Python 3.9 and Linux Podman-volume Python 3.11. Applying 11
focused tests with 14 branches to the old driver produces 13 contract failures:
ten behavioral failures and three checks for the new atomic-write mechanism. The
remaining serialization guard was already working. A separate partial-write
fault against the old stream also leaves a seven-byte, unparseable JSON file.
Run the suite without native artifacts or Podman in both modes:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/agent-triage/test_sampling.py
PYTHONDONTWRITEBYTECODE=1 python3 -O benchmarks/agent-triage/test_sampling.py
```

The suite now also checks optimized-interpreter entry points and response
correctness. The measurement, storage A/B, caller profiling and volume lifecycle
entry points reject `-O`, `-OO` and `PYTHONOPTIMIZE` before creating output or
state directories, building artifacts or starting helper processes because the
full workloads still depend on assertions. The imported `request()` helper
uses explicit checks for admission, polling, terminal results, adapter attempts
and micro responses, so optimization cannot silently accept a wrong response.
The main-loop fault tests bypass only the separately tested interpreter guard;
they still exercise persistence and teardown under optimized Python.

The full frozen-protocol matrix passed all 132 budget checks and 115 process
cleanups on the local Linux ARM64 Podman named-volume target with the final
driver. `report.py` independently recalculated the raw result; replaying its
compressed archive reproduced the summary and report byte for byte. This is a
sampling-reliability acceptance result, not a claim of performance improvement.
Driver identity, old-behavior reproductions and helper-fault audit are retained in the
[O3 evidence](../../docs/agent-execution-evidence/README.md#validation).
The protocol, native binaries, application TypeScript and deployment helper match
O2. Its 34 product regression groups and cross-container lifecycle results remain
historical; these unchanged paths were not rerun for O3. Application behavior,
workload, budgets and historical evidence remain outside this change. Normal
cold, warm and recovery timing boundaries remain the same.
The total-run `durationSec` metadata now includes helper initialization; that
field does not participate in the 132 budget checks.

## Dispatcher throughput diagnostic (O4, local retention accepted)

The user selected higher concurrent throughput. The candidate Python supervisors
wake on HTTP 200 / `status=completed`, refill available slots and scan every 50 ms
for a 250 ms follow-up window. Idle or unavailable scans use the original 250 ms
fallback; unresolved jobs retain their own 250 ms retry floor. Eight slots,
5 ms status observation, application/native code, SQLite, permissions, audit,
physical-attempt limits, leases and deadlines are unchanged.

`dispatch-ab.py` runs the [frozen plan](fixtures/dispatch/o4-predeclared.json):
two payloads and two adapter delays, four adjacent pairs per c4 cell and two per
c1 guard cell, rotating cells and alternating AB/BA for 24 pairs / 48 rounds.
Each round has a fresh whole namespace on the same volume, eight warmups and
32 measured jobs. Both sides execute their own exact packaged `deploy.py` through
the reused O3 sampler. Caller, plugin and worker artifacts must be byte-identical.

Identical HTTP observers record route class, duration, status and optional job ID,
never credentials. Control requests per job count pending plus dispatch attempts
over all 40 jobs, including already-started calls during shutdown; this is not a
measured-jobs-only metric. Allowed internal 503 responses and shutdown cancellation
remain visible. Runtime exceptions, incomplete coverage or cleanup cannot produce
a valid selection. Live measurement requires normal Python so reused workload
assertions remain active; offline validation also works under `-O`.

```sh
python3 benchmarks/agent-triage/dispatch-ab.py \
  --bin-dir /tools/build/release \
  --baseline-release /volume/o3-20260928/matrix-1/releases/lookup \
  --candidate-release /volume/o4-20260928/candidate-derived/releases/lookup \
  --output /volume/o4-20260928/ab-new \
  --plan benchmarks/agent-triage/fixtures/dispatch/o4-predeclared.json
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/agent-triage/test_dispatch_ab.py
PYTHONDONTWRITEBYTECODE=1 python3 -O benchmarks/agent-triage/test_dispatch_ab.py
```

AB2 passed all paired rules and independent raw-data recomputation. The four c4
cells show median paired throughput gains of 60.8–90.9%, median paired p95
reductions of 32.2–43.8%, and median paired control-request increases of
68.1–85.2%. These local observed-pair results are not production SLOs or a claim
of statistical significance. AB1 collected all 48 rounds but retained an `error`
after exact float equality rejected a one-ULP Python/Rust JSON round-trip
difference. The check now preserves sample length/order and uses the existing
numeric tolerance; candidate/thresholds were unchanged and AB2 reran every round.
Ten paired-tool test groups pass host/Linux normal and optimized Python and are
wired into CI; remote CI has not run.

`eligibleForFullValidation` only advances the candidate to the original
uninstrumented matrix, related regressions and exact-package lifecycle. All those
gates now pass: 132/132 original cost checks, 115 process cleanups, 34 regression
groups, and two-container/off-volume recovery with the exact measured package.
Recovery takes 45,572.73 ms against the 50,000 ms budget, with healthy/crash reads
of 1/2 and zero added reads on old-key replay. Both supervisor changes are retained
for this local target, with the measured control-traffic cost preserved above.
The paired record's `retainCandidate` remains false because the diagnostic cannot
make the final decision; the final
[retention gate](../../docs/agent-execution-evidence/README.md#dispatcher-throughput)
records `accepted` / `retainCandidate=true`. This adds no production SLO or
power-loss guarantee. Exit 0 means paired gates pass, 2 means valid completed
pairs miss a gate; exceptions retain error evidence and exit nonzero. See
[O4 evidence](../../docs/agent-execution-evidence/README.md#dispatcher-throughput).

## Caller diagnostic (P5.1)

`profile-caller.py` copies a caller source tree, wraps its SQLite calls, packages
that temporary copy and runs three bounded, four-client lookup rounds. Temporary
HTTP response headers report SQL class/count/elapsed time; the retained native
logs also show capability timings. The runtime does not provide `console`, so
instrumentation must not assume it. Production source and the frozen acceptance
driver are not instrumented.

```sh
python3 benchmarks/agent-triage/profile-caller.py \
  --bin-dir target/release --source examples/agent-triage/src \
  --output target/agent-caller-profile
```

Headers attribute the **HTTP caller path**, not background Durable SQL. The
per-call wall times include queueing and have millisecond resolution; sums from
concurrent requests are not CPU time or end-to-end wall time. Instrumentation and
header parsing add overhead. The same closed-loop poll can send more status
requests when they become faster, so compare both call counts and job throughput.
These runs are diagnostics, not substitutes for the full uninstrumented matrix.

Both this profiler and `storage-ab.py` mark a run complete only after helper
shutdown and final metadata/log writes succeed. A teardown failure records
`status="error"` and cleanup details, retaining any earlier measurement error.
Final JSON writes are attempted even after another teardown fails; if the write
itself fails, stderr retains all errors and the last atomic snapshot remains.
Offline regressions: `python3 benchmarks/agent-triage/test_diagnostic_cleanup.py`.

For a separately labeled filesystem comparison, add `--state-root /tmp` inside
the same Linux container. Only temporary namespace/config/database files move;
the packaged executables and retained observations stay under `--output`.
The diagnostic records that alternate root and removes it on exit. Its result
does not satisfy the original bind-mount budgets, prove power-loss durability,
or change the deployment's backup/restore contract.

## Podman storage comparison (P5.2)

`storage-ab.py` compares the exact retained P5 and P5.1 packages without modifying
their TypeScript. The [predeclared plan](storage-plan.json)
specifies eighty rounds: two payloads, five config/application-SQLite/Durable-SQLite
placements and four adjacent A/B pairs per cell. Both variants use the same
directory-link mechanism and explicit existing Durable path override. The script
checks actual open database paths and mount identities outside the timer, then
reuses the original warm-round sampler, outputs, attempt checks and cleanup.

These split layouts are diagnostics only: the normal backup helper does not
follow an externally overridden Durable path. Native log summaries are explicitly
limited to the retained last 200 lines and cannot measure whole-round SQL counts.
Storage effects can interact; single-component contrasts cannot be added together.
The report treats each A/B pair as a repeat, not each job as independent evidence.

```sh
python3 benchmarks/agent-triage/storage-ab.py \
  --bin-dir /tools/build/release \
  --baseline-release /tools/run-1/releases/lookup \
  --candidate-release /prior/run-1/releases/lookup \
  --bind-root /out/ab-bind --volume-root /volume/ab-volume \
  --output /out/ab \
  --plan benchmarks/agent-triage/storage-plan.json
python3 benchmarks/agent-triage/storage-report.py \
  --input /out/ab/measurements.json --output /out/ab-report
```

For the complete normal deployment matrix on the separately named Podman volume
target, use `run.py --protocol` with
[protocol-volume.json](protocol-volume.json).
Only `environment.storage` differs from the original protocol. All budgets and
workload fields stay fixed. When its output is on the volume, both packaged
artifacts and temporary namespaces are there, so cold-start differences include
executable loading and verification. This result never erases the original
bind-mount gate's failure.

`volume-lifecycle.py` verifies original artifact/state identity across two
successive containers mounting the same named volume. Run `--phase prepare` then
`--phase resume` immediately, with separate output files, the same original
`--release` and the same `--state-root`. `--backup-root` on resume can name a new
directory on the separate evidence mount to check stopped export/restore across
filesystems. Normal deployment, default lease and backup helpers are used; no
split-layout override is applied. The report separately records lifecycle
correctness and the recovery time including the container gap. It retains state
for inspection and does not test VM reboot or power loss.
