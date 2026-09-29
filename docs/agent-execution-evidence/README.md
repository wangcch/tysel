# Agent iteration results and evidence

This index records the retained changes, their measured tradeoffs and unresolved
risks. Measurements used a local Linux ARM64 Podman VM with 8 vCPU / 8 GiB;
Agent deployment measurements used its named volume. They are engineering
observations, not production SLOs or a comparison against the clean v0.3.0 tag.
The different stages have different baselines; their percentages cannot be added.
Later diagnostic fixes changed native binary identities, so these results are
not a new unified performance measurement of the final submitted commit.

## Storage target

The original bind-mount matrices missed 14/132 checks (P5) and 15/132 (P5.1).
The combined schema/maintenance-precheck candidate was withdrawn. The separate
Podman named-volume target passed 132/132 checks, without changing numerical
budgets, plus exact-package cross-container recovery and stopped backup/restore
to a different filesystem. This does not establish power-loss durability or
resolve the bind-mount misses.

The three complete cost observations, expected summaries and reports remain as
CI fixtures. Their failed and successful results are preserved byte for byte.
The stable `benchmarks/agent-triage/protocol-volume.json` changes only the storage
description from the original protocol. `storage-plan.json` preserves the
historical paired-storage experiment, including its exact artifact identities.

## Dispatcher throughput

Completion-triggered refill was compared with the exact O3 packaged supervisor
in 24 adjacent balanced pairs / 48 rounds across eight cells. At four clients,
cell medians of paired throughput ratios improved **60.8–90.9%** and end-to-end
p95 ratios fell **32.2–43.8%**. Control requests per job increased **68.1–85.2%**.
The final local retention record is `accepted` / `retainCandidate=true`, after
132/132 full-matrix checks, 115 cleanups, 34 related regression groups and the
exact package's cross-container / off-volume lifecycle passed.

The earlier AB1 attempt remains an error: a one-ULP JSON float round-trip
difference failed exact equality after sampling. The validator was corrected
with regression coverage and AB2 repeated the full experiment. A diagnostic
record's `eligibleForFullValidation=true` is not final acceptance.

Eight dispatcher slots and unresolved-job retry spacing remain bounded. The
separate c4-to-c8 experiment gained only -0.1% to +9% throughput while p95 rose
75–121%; it does not justify increasing the concurrency limit.

## Web API bytecode cache

The cache reuses the fixed embedded script's compiled bytecode within one
process; JS objects, globals and host bindings remain fresh per isolate.
Balanced B/C/C/B/B/C full suites compared it with the pre-cache O4.1 binaries.
Figures are medians of three per-round statistics, not pooled percentiles.

| Metric | Before | Candidate | Change |
| --- | ---: | ---: | ---: |
| Durable resume p95 | 2.402 ms | 1.245 ms | -48.2% |
| Durable suspend p95 | 2.371 ms | 1.204 ms | -49.3% |
| Replay 1,000 effects p95 | 4.671 ms | 2.674 ms | -42.8% |
| Whole-suite guest CPU | 18.376 s | 16.932 s | -7.9% |
| Cold startup p50 | 3.359 ms | 3.813 ms | +0.454 ms / +13.5% |
| Idle service PSS | 7.152 MiB | 7.504 MiB | +0.352 MiB |
| Growth after 1,000 isolate reuses | 1,804 KiB | 2,040 KiB | +236 KiB |

All 54 original numerical gates passed. Cold startup passes the predeclared
absolute 0.5 ms allowance; it exceeds 10%. An initial HTTP-tail flag required
six additional balanced pairs under a frozen rule; those resolved the flag.
Both the initial results and follow-up remain archived.

## Caller state maintenance

Caching successful schema initialization per isolate reduces the migrated
status path from 12 SQLite calls to four. Authority/configuration still reload
and all three maintenance updates still execute in order on each request.
Failed or interrupted initialization is retried on a later request.

Across four cells and 24 paired rounds, geometric means of cell median ratios
show **14.0% lower native CPU/job**, **1.4% higher throughput**, and **3.8% higher
end-to-end p95**. The primary benefit is CPU cost.

Three long-memory pairs recorded caller peak PSS as follows:

| Pair | Baseline | Candidate |
| --- | ---: | ---: |
| 1 | 57.12 MiB | 57.05 MiB |
| 2 | 150.66 MiB | 54.91 MiB |
| 3 | 147.72 MiB | 56.17 MiB |

Median paired peak and quiet-60 ratios fell 62.0% and 58.1%. The low first
baseline shows that the high point is variable; these three pairs do not prove
a universal reduction or leak freedom. Attribution found substantial allocator
free space, without identifying an exact allocation stack. Two later complete
cost matrices each passed 132 checks / 115 cleanups, plus 35 Agent groups and
the exact package's lifecycle checks.

## Validation

The latest native diagnostic fixes passed 294 host tests and 296 Linux ARM64
release tests, each with one existing ignored measurement, plus focused
classification regressions, Clippy, formatting and packaged acceptance. Later
Python-only cleanup fixes passed normal and optimized fault tests on both hosts.
These are historical local validations of recorded source/binary identities;
the final submitted commit still requires full CI before merge.

Three confirmed review findings were fixed: ordinary messages containing
`memory` were misclassified as OOM; optimized Python could disable measurement
assertions; diagnostic teardown failures could leave a false `complete` status.
The fixes preserve runtime budgets and fail closed on incomplete diagnostics.

One original normal-budget snapshot startup failed with a generic QuickJS
exception. It occurred at the **second snapshot startup**, correcting the first
wording in the original caller report. The original trigger remains unknown;
successful reruns and artificial low-budget probes do not establish its cause
or prove it fixed. No higher limits or retries were used to hide it. Task
inspection is a code-supported location inference, not a recovered stack trace.

## Repository and archive layout

Git contains this summary, the [archive index](archive-index.json), reusable
benchmark/acceptance tools, 12 byte-identical offline fixtures under
`benchmarks/agent-triage/fixtures/`, and stable protocol inputs. See the
[fixture provenance and replay commands](https://github.com/wangcch/tysel/blob/main/benchmarks/agent-triage/fixtures/README.md).
CI and offline tests do not require the historical archive or measured packages.

The complete 1,223-file dated evidence tree, including failures, original source
snapshots, protocols, seven manifests and process logs, is preserved in the
ignored `local-artifacts/agent-execution/history.tar.gz`. Eight pre-cleanup
documentation snapshots are also included. `ARCHIVE-MANIFEST.json` inside the
bundle hashes every preserved source file. Historical manifests were not edited.
The index records the bundle SHA-256 and selected source records for the numbers
above. Its SHA verifies the bundle; it does not make the observations a production
guarantee.

**The bundle is currently local only.** A clean clone can replay the committed
cost fixtures and run the offline tests; it cannot recompute every historical
performance claim without obtaining the archive and, for live repetition, the
separately retained exact packages. No shared download has been published. When
sharing the history, provide the verified bundle and update the index with its
actual location. Do not remove the local copy before a shared copy is verified.
The bundle does not contain native binaries or complete packaged releases.

To inspect an available local bundle, first compare its hash with the index:

```sh
shasum -a 256 local-artifacts/agent-execution/history.tar.gz
mkdir -p target/agent-history-review
tar -xzf local-artifacts/agent-execution/history.tar.gz -C target/agent-history-review
```

Extract into a separate directory: archived document paths would overwrite the
current summaries if extracted over the checkout. Historical one-off scripts may
require their recorded workspace or container layout. New run output belongs in
`target/` or `local-artifacts/`; promote only necessary fixtures, reusable tools
and concise reviewed results into Git, preserving failures with each full run.
