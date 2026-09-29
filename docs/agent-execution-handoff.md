# Agent execution handoff

The bounded read-only application tranche is complete for the local Linux ARM64
Podman named-volume target. Retain the dispatcher, Web API bootstrap cache and
caller schema-initialization reuse. The latest review has no open concrete code
findings; **merge remains conditional on full CI passing for the final submitted
revision**. Local passes are not a release-wide or production-readiness claim.

Use the [iteration contract](agent-execution-iteration-plan.md) for scope and
invariants, the [improvement plan](agent-execution-improvement-plan.md) for next
work, and the [evidence summary and archive index](agent-execution-evidence/README.md)
for measurement provenance. Daily logs and superseded handoffs are archive
material, not part of the maintained decision record.

## Retained results and tradeoffs

These are separate stage comparisons against their own exact baseline artifacts.
Do not add the percentages or describe them as one comparison against v0.3.0.

| Change | Observed result | Cost or limit |
| --- | --- | --- |
| Podman whole-namespace placement | P5.2 passes 132/132 frozen cost checks, packaged regression and cross-container/off-volume restore | Local named-volume result only; P5/P5.1 bind mounts still miss 14/15 checks |
| O4 dispatcher wakeup/refill | Four-client median paired throughput +60.8–90.9%; end-to-end p95 −32.2–43.8% | Control requests/job +68.1–85.2%; local closed-loop workload |
| Web API compiled-byte cache | Durable resume p95 2.402 → 1.245 ms (−48.2%); whole-suite guest CPU −7.9% | Cold start +0.454 ms (+13.5%), idle PSS +0.352 MiB, reuse growth +236 KiB |
| Successful schema initialization reuse | Successful status path 12 → 4 SQLite calls; aggregate native CPU/job −14.0% | Throughput +1.4%, p95 +3.8%; retain as a cost reduction |
| Caller long-memory pairs | Candidate peaks 54.91–57.05 MiB in three pairs | Baselines were 57.12, 150.66 and 147.72 MiB; no universal 62% reduction or leak-fix claim |

The dispatcher wakes on completed work and briefly follows up at 50 ms, retaining
250 ms idle and per-job retry floors. Concurrency, polling, authorization and
attempt limits are unchanged. The cache shares fixed compiled Web API bytes;
each runtime still creates its own objects and host bindings. Schema reuse
caches only successful initialization per isolate. Configuration, authority and
ordered maintenance remain live, and failed initialization remains retryable.
The earlier combined schema/maintenance candidate was rejected and withdrawn;
it is distinct from the later retained schema-only change.

The capacity diagnostic does not justify raising concurrency: four to eight
clients changed throughput by about −0.1% to +9% while p95 rose 75–121%.
Memory instrumentation supports allocator free-space retention as a contributor
to high resident memory, but does not identify a precise allocation stack or
prove absence of leaks. The historical v0.3.0 latency difference is not fully
attributed by finding and reducing bootstrap cost.

## Correctness and validation

The iteration also makes saved reports recompute results from raw samples,
cleans exclusively created backup/restore destinations after caught failures,
and bounds sampler communication and teardown. Final review closed three P2s:

- Ordinary errors containing `memory` no longer become MemoryLimit merely from
  that text. Typed allocation errors and the engine's exact InternalError OOM
  diagnostics are recognized; other exception messages and stacks survive.
- Measurement entry points reject optimized Python before side effects, and
  imported request validation uses explicit checks that survive `-O`.
- Storage A/B and caller profiling only save `complete` after successful
  measurement and cleanup; primary, cleanup and final-save errors stay visible.

HTTP worker startup and isolated task inspection now preserve exception detail,
operation/phase and timeout classification. These repairs improve diagnostics;
they do not establish new performance gains or explain the historical trigger.
An engine-like InternalError can still be constructed by JS, and severe OOM can
throw null, so memory classification is not allocator-proven provenance.

The latest native source passed 294 relevant macOS and 296 Linux ARM64 tests,
Clippy and formatting, seven direct worker controls, 35 Agent groups, three
example workflows and standalone HTTP. The later diagnostics-only Python fix
passed eight cleanup tests on both platforms in normal and optimized modes,
along with the real child-interpreter guards. These stage-specific local results
are recorded in the evidence index; none substitutes for final-revision full CI.

The retained caller package separately passed two complete 132/132 cost matrices,
115 cleanup checks per matrix, and exact-package lifecycle validation. Later
native diagnostics changed the binary bytes. Earlier performance measurements
remain attached to their measured artifacts; no unified performance A/B has
been run for the final native binaries.

## Remaining risk and next action

One original full-matrix attempt failed at the **second snapshot startup** under
normal limits, before budget evaluation. The original exception lost detail.
Six hundred subsequent startup/request checks did not reproduce it; deliberately
low-limit probes reproduced generic errors, not the original normal-budget event.
The trigger remains unknown. It is neither proven introduced by the cache nor
proven fixed by better diagnostics. Retain it for release review and capture the
new detail if it recurs; do not mask it with retries or higher limits.

Run required full CI against the submitted candidate before merge. For a release
or a new performance claim, bind acceptance and measurements to the actual final
artifacts. Follow the improvement plan for startup investigation and remaining
maintenance cost work. Preserve exact packages with their bound namespaces;
never replace their supervisor or rebuild them in place.

Operational constraints are in the [deployment guide](../examples/agent-triage/DEPLOYMENT.md).
The result still assumes one caller, one application writer and one Durable
scheduler, fake development identities and a local read-only provider. Required
Linux isolation setup must succeed. Stopped, terminal, drained backup and restore
to a new directory do not provide live backup, distributed fencing or power-loss
durability. No generic Agent API or real-provider integration is implied.
