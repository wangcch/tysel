# ADR-011: Keep bounded Agent coordination in the application

Status: Accepted for the example boundary, 2026-09-24. P5/P5.1 bind-mount cost
gates **failed**. P5.2 passes **132/132** cost checks on the selected local Podman
named-volume target; cross-container lifecycle and all 34 final regression
groups passed. P5.2 is complete within that local scope.
This decision does not accept performance for production.

## Decision

Keep the read-only ticket workflow in `examples/agent-triage`, using existing
HTTP, isolated profiles, capability grants and Durable effects. Do not introduce
a generic Agent API, a broker or a reusable application library from this one
synthetic workflow. End the abstraction investigation at this boundary. P5.2
resolves the local named-volume cost target; the historical bind-mount misses
remain failures for that different storage path.

The final P5.2 lifecycle, correctness and cost closing conditions are met for
the documented local Podman named-volume target. Close this bounded tranche at
the application boundary. No frozen budget was relaxed, and no cost miss
establishes the need for a new runtime API.

## Evidence and limits

P1–P4 established local admission, scope checks, bounded physical attempts,
crash recovery, hostile-message handling, audit and exact-artifact Linux delivery.
P5 tested release builds on an Apple M5 Pro-hosted Linux ARM64 VM, 8 vCPU / 8 GiB,
using workspace bind-mounted SQLite files. This is a named local engineering
experiment, not dedicated-host publication data, x64 coverage or production load.

The predeclared protocol used one or sixteen tickets, 64- or 500-byte subjects,
one/four clients, zero/20 ms synthetic provider delay, 60 cold samples, 1,536 warm
jobs in three rounds per cell, 384 warmups and three default-lease crashes.
All results, physical-attempt counts, expected denials and cleanup assertions
passed. Of 132 numerical checks, 14 failed:

- All twelve four-client lookup throughput rounds produced 4.79–5.54 jobs/s,
  below the declared 6 jobs/s minimum.
- Two of those rounds had p95 latency of 1,053.76 and 1,020.84 ms, above 1,000 ms.
  Pooling each cell's three rounds hides these two tail misses, so the retained
  decision uses the per-round checks too.
- Sequential lookup met the 2 jobs/s and 1,000 ms p95 budgets. Direct and isolated
  snapshot warm p95 values were at most 3.36 and 3.48 ms, respectively.
- Cold p95 was 36.27 / 54.70 / 768.44 ms for direct / snapshot / lookup, below
  3,000 ms. Sampled peak native PSS was 9.51 / 12.61 / 51.18 MiB, below the
  declared 256 / 256 / 512 MiB bounds. Python and the measurement helper are
  excluded; these are not whole-deployment memory totals.
- Recovery took 45,455.15 / 45,400.53 / 45,381.11 ms with the original 45-second
  lease, below 50,000 ms, and exactly two authorized reads per crash case.
  Three samples establish no recovery tail estimate.

Direct/snapshot already receive projected detail. Lookup additionally performs
authorization, a read, persistence, journaling, outbox scheduling and status
polling. Its higher cost cannot be attributed solely to isolated IPC. Audit
timestamps show substantial queueing and intervals between recorded operations;
they do not distinguish filesystem latency from SQLite serialization, Durable
bookkeeping or status-load effects. A causal claim requires profiling.

The repository retains the frozen protocol, cost-test fixtures and a concise
result index. Full process logs and historical identities are in the local
archive described by that index:

- [Measurement method](https://github.com/wangcch/tysel/blob/main/benchmarks/agent-triage/README.md)
- [P5 evidence index](../agent-execution-evidence/README.md#storage-target)
- [Full results and misses](https://github.com/wangcch/tysel/blob/main/benchmarks/agent-triage/fixtures/cost/p5/results.md)

## P5.1 follow-up result

The [P5.1 evidence](../agent-execution-evidence/README.md#storage-target) verifies
schema reuse per HTTP isolate and a read-only maintenance precheck, with the
original three ordered updates when needed. All 34 correctness groups passed,
including initialization failure/retry, concurrent legacy migration and a
648-row deadline/ownership/delivery matrix. Authority remains fresh; audit,
physical attempts, lease and durability settings are unchanged.

Diagnostics reduced HTTP schema calls from 17,746 to 234 across the same number
of jobs, and native SQLite exec calls from 24,098 to 2,127. Faster status requests
also increased polling traffic. The full uninstrumented, unchanged bind-mount
matrix failed **15 of 132** checks: all twelve four-client throughput rounds
(3.81–5.46 jobs/s), one cell p95 (1,360.49 ms) and two round p95s (1,205.19 and
1,536.81 ms). Cold, memory, sequential, recovery, output, attempts and cleanup
checks passed. The run observed worse tails than P5 and does not establish an
end-to-end improvement. The SQL reduction was retained as experimental, not
accepted performance work at that stage; P5.2 subsequently withdrew it under a predeclared
selection rule. The original failed baseline remains visible.

A separately instrumented container-local namespace/config/database experiment
reached 15.71–16.05 jobs/s. It supports investigating the storage boundary, not
replacing the failed bind-mount gate or attributing all delay to one SQLite
operation. The combined-UPDATE candidate was rejected; WAL was not run or enabled.

## P5.2 Podman target and selection

The user selected Podman as the next deployment target. Eighty diagnostic rounds
compared P5 and P5.1 across five storage placements and two payloads, using four
adjacent pairs per cell with balanced AB/BA order. The predeclared rule required
both whole-volume payloads to have 4/4 nondecreasing candidate throughput pairs,
median rate ratio >= 1.05 and median p95 ratio <= 1. Both had only 2/4
nondecreasing pairs; median rate ratios were 0.9963 / 1.0101 and p95 ratios
1.1910 / 1.1613. The candidate did not qualify and was withdrawn. The final
application restores P5 `store.ts`; reduced SQL call counts alone did not justify
retaining the optimization. This is an engineering decision, not significance.

The final uninstrumented matrix put the entire namespace on a Podman named volume
in the local Linux ARM64 VM. All **132/132** frozen cost checks passed: 60 cold
samples, 1,536 measured warm jobs, 384 warmups and three default-lease crashes.
Four-client lookup throughput was **11.05–15.48 jobs/s**, the largest lookup cell
p95 **421.63 ms**, and lookup cold p95 **450.60 ms**. Recovery was
**45.41–45.45 seconds**. Output, attempt, denial and cleanup assertions passed.
The 5 ms status poll, 250 ms outbox, 45-second lease and budgets were unchanged.

This accepts cost only for the named local Podman volume target. It does not
rewrite P5/P5.1 bind-mount failures, establish a deployment-wide memory total,
or extrapolate to production, x64, power loss or other storage. Split-store
diagnostics are conditional comparisons, not additive causal attribution or
supported deployment/backup layouts. Normal deployment keeps both databases,
configuration and binding in the same persistent namespace.

Cross-filesystem restore exposed copying of source security attributes as an
application delivery bug. Packaging now preserves exact worker bytes and
executable mode, and stopped backup/restore copies bytes and POSIX modes without
source SELinux labels, ACLs, ownership or timestamps. It rejects symlinks/special
files and restore destinations inside the snapshot. The focused cross-filesystem
healthy-job probe passed with equal records/binding and no extra read. All 34
final regression groups (8 contract, 8 recovery, 8 adversarial, 6 deployment and
4 initialization) passed with temporary state on the named volume.

Two new containers reused the identical measured package and namespace; recovery
took **45,339.36 ms**, including the container handoff, with the original lease
and deadline. Original job identity and the consumed slot survived. Independent
reads totaled one healthy and two crash-job reads. Stopped backup exported to a
host bind mount and restored into a new volume directory retained database
records and artifact binding; old keys added no reads. Cleanup passed. The
first lifecycle driver wrongly required an unacknowledged NULL outbox `task_id`
to remain unchanged, and failed after successful recovery. Both the failure and
first driver are retained; only the verifier was corrected to check independent
identity and unique execution, without changing production or measured packages.
See the
[P5.2 evidence](../agent-execution-evidence/README.md#storage-target).

## Follow-up boundary

P5.2 closes this local tranche; there is no further generic Agent stage.
Any separately scoped optimization or deployment target must preserve authority, audit,
version, attempt and recovery contracts and rerun its affected acceptance plus
the frozen cost matrix. No authority cache, attempt refund, shortened lease,
weaker synchronization, WAL or reduced status traffic was introduced for P5.2.

Reopen abstraction work only when:

1. Independent real applications duplicate stable caller logic; evaluate a small
   application helper before changing the runtime.
2. A required capability demonstrably cannot be implemented correctly or
   affordably with existing APIs after profiling and application alternatives.
   Any runtime proposal must include the failing workload, threat model,
   compatibility impact, alternatives and rollback.

This ADR adds no public runtime API or compatibility obligation. The existing
artifact/namespace binding and stop-before-backup contract remain in force.
