# Agent execution scope and validation contract

This document records the durable scope and acceptance contract of the bounded
Agent example. P0–P5.2 are complete within the local evidence described below.
Current optimization results and merge readiness live in the
[handoff](agent-execution-handoff.md); future work lives in the
[improvement plan](agent-execution-improvement-plan.md). Measurement provenance,
failed attempts and archive retention are summarized in the
[evidence index](agent-execution-evidence/README.md).

## Objective and stopping point

Validate whether a trusted Tysel service can coordinate one bounded, untrusted
TypeScript plugin with existing HTTP, isolation, capability and Durable APIs.
The workload is a read-only customer-ticket lookup using fake development
identities and a local adapter. Keep policy, credentials, budgets, audit and
business state in the trusted application.

The result is the runnable [Agent triage example](../examples/agent-triage/README.md),
reproducible tests, benchmark tools and an explicit
[architecture decision](adr/011-bounded-agent-application.md). This single
synthetic workflow does not justify a generic Agent API or application library.
Real providers, production identity, multiple replicas, external writes, UI
orchestration, model routing and general subprocess hosting require new scope.

## Stage decisions

| Stage | Delivered capability | Local decision |
| --- | --- | --- |
| P0 | Consolidated durable and isolated baseline tests | Passed the selected development checks |
| P1 | Runnable caller, persisted admission and bounded protocol | Contract groups C01–C06a passed |
| P2 | Durable continuation, stable outbox admission and physical-attempt reservations | Recovery groups C07–C09 passed |
| P3 | Hostile messages, customer boundaries, current authorization and secret exclusion | Adversarial groups passed |
| P4 | Exact packaged artifacts, namespace binding, Linux isolation and stopped restore | Local Linux ARM64 delivery passed |
| P5 | Frozen costs and architecture decision | Bind-mount cost gate failed: 14/132 checks missed |
| P5.1 | Combined schema/maintenance candidate | Correctness passed; bind-mount cost gate failed: 15/132 missed |
| P5.2 | Whole-namespace Podman volume, controlled SQL selection and lifecycle verification | SQL candidate withdrawn; 132/132 cost checks, 34 regression groups and lifecycle passed |

The named-volume pass is for a different storage target; it does not turn the
bind-mount failures into passes. Split-store experiments are diagnostics, not
supported layouts or additive cost attribution. Later optimization stages retain
this contract; their source and artifact identities have separate evidence.

## Working architecture

```text
Request with service-owned identity
  -> trusted service: admission, policy, budget, durable state
  -> isolated plugin: projected data -> result or operation request
  -> trusted service: validate operation, reserve attempt, recheck authority
  -> configured read-only adapter
  -> isolated plugin: projected response -> final output
  -> trusted service: validate and persist result
```

The namespace has one caller, one application SQLite writer and one Durable
scheduler. A Python supervisor drains admitted work and reserves service capacity
for admission/status while executions await I/O. It is part of the example's
package. Plugin-supplied tenant IDs, URLs, budget claims, flags or final text can
never grant authority. Credentials and unrelated customer fields stay outside
plugin traffic, durable input/history, results and diagnostics.

## Application invariants

### Admission and identity

- Bind loopback development credentials to service-owned customer scopes.
  `POST /jobs` requires an `Idempotency-Key`; `GET /jobs/:jobId` authorizes status
  and result access. Another customer's job is indistinguishable from missing.
- Persist admission before acknowledgement or dispatch. The same trusted
  customer/key and canonical input resolve to one job; conflicting input returns
  409. Resolve retained admissions before applying new-job capacity limits.
- Bind original input, protocol, caller program, plugin digest and adapter
  identity to the job. Uncertain admission retries the original key and bundle.
  Neither restart nor deployment silently substitutes new identity or code.
- Client disconnect does not cancel admitted work. There is no cancellation
  endpoint in this tranche. Old P1 jobs keep their interruption/expiry semantics;
  newly admitted P2 jobs use bounded recovery.

### Execution, authorization and recovery

Business states are `accepted`, `running`, `recovering`, `succeeded`, `failed`
and `expired`; business outcome and Durable completion are distinct. A valid
conditional terminal write wins once. Old attempts cannot replace its result,
refill quota or revive a failed/expired job.

Strict application envelopes bind `protocolVersion`, `jobId`, `stepId` and
`attemptId`. Reject unknown fields, malformed or stale messages, extra logical
lookups and forbidden operations. The caller owns the pending operation; echoed
identifiers alone do not authorize it. Final text remains untrusted data.

Each physical attempt reserves a durable application slot before dispatch. An
unknown dispatch outcome never refunds it. At the actual dispatch boundary,
recheck current service policy, fresh time, original deadline and attempt owner;
recorded authorization or time from an earlier effect is insufficient. A read
already in flight may finish after revocation or timeout and cannot be undone.

Application state and Durable history remain separate stores. Stable outbox keys
and original inputs repair acknowledgement gaps without claiming a cross-store
transaction. Unrecorded reads may repeat within the attempt bound and observe
newer data. Recorded detail and accepted results are reused unchanged. Runtime
history fencing alone is not authority over a separate business database.

### Limits and retention

These are application defaults, not performance promises. The smaller runtime
limit still applies; byte bounds use serialized UTF-8 and are enforced during
response reading, before parsing.

| Limit | Contract |
| --- | --- |
| Tickets / subject | At most 16 tickets, 500 characters per subject |
| Admission / adapter response | 16 KiB each |
| Plugin request / response | 16 KiB / 8 KiB including envelope |
| Logical plugin calls / privileged reads | One plugin call for empty input, otherwise at most two; at most one `ticket.read` |
| Physical attempts | At most two per logical call in recoverable jobs |
| Per-call timeout / total lifetime | 5 seconds / original 120-second deadline |
| Retry delay | One 250 ms delay within the original deadline |
| Active jobs | Four per customer, eight total; excess new admissions return 429 |
| Retained admissions | 1,000 total; new keys fail explicitly at the cap |

Restart, retry and lease waiting consume the original deadline and attempt
budget. Enforce response and input limits, reject redirects and use only
configured origins. Expiry is materialized when storage and the scheduler are
available; no immediate durable write during an outage is promised.

There is no automatic pruning or reset command. Retain admissions, receipts,
results and original artifacts together. A new namespace has no cross-reset
idempotency guarantee. Any future cleanup must first define a retry horizon,
tombstones and interaction with runtime admission pruning.

### Delivery and restore

A namespace binds exact caller/plugin/worker/supervisor artifacts and has one
owner lock. Rebuilding may change bytes; new releases use new namespaces rather
than replacing bound artifacts. The source demo removes its temporary namespace
on exit, while the packaged deployment keeps state.

Back up only stopped, terminal, drained state, with other producers fenced.
Restore both stores and binding to a new directory using original artifacts.
Copy bytes and POSIX modes; destination filesystems own labels/ACLs/ownership.
Reject symlinks, special files and snapshot-internal restore destinations.
Cleanup after caught failures touches only the operation's exclusively created
target. This does not provide live backup, arbitrary filesystem-race protection,
power-loss durability or rollback-safe replay of later external reads.

Required Linux Landlock, seccomp and rlimit setup must succeed; cgroup control
is best effort. macOS development passes do not prove Linux isolation. See the
[deployment guide](../examples/agent-triage/DEPLOYMENT.md) and the existing
[Durable recovery contract](reference/runtime/durable.md#active-crash-recovery).

## Acceptance matrix

Reuse the [acceptance runner](../tests/acceptance/README.md) and its exact-tool,
namespace and process-cleanup conventions. Cases must exercise observable fault
boundaries, persisted state and independent adapter counters. A timeout or cleanup
failure fails the run; missing history is never inferred as successful execution.

| Cases | Required observation | Test source |
| --- | --- | --- |
| C01–C02 | Empty/populated output; concurrent same-key requests create one job; conflict returns 409 | `tests/acceptance/agent_triage.py` |
| C03–C04 | Customer separation; forbidden/malformed operations cause zero unauthorized reads | `tests/acceptance/agent_triage.py` |
| C05–C06a | Exact/over limits, expiry, retained restart identity and lost admission response | `tests/acceptance/agent_triage.py` |
| C07 | Unrecorded reads may change; recorded detail/result stay fixed | `tests/acceptance/agent_triage_recovery.py` |
| C08 | Crashes around admission, reservation, dispatch, effect history and result commit preserve bounds | `tests/acceptance/agent_triage_recovery.py` |
| C09 | Stale owners cannot win; retries use fresh authorization and consume reserved attempts | Recovery and adversarial suites |
| C10 | Hostile plugins, cross-job messages, canaries and streamed limits preserve the trust boundary | `tests/acceptance/agent_triage_adversarial.py` |
| C11–C12 | Exact-package Linux healthy/crash/denial behavior; retained keys, binding and stopped restore | `tests/acceptance/agent_triage_deployment.py` |
| Initialization | Failure/interruption is retryable; authorization and maintenance stay live | `tests/acceptance/agent_triage_initialization.py` |
| C13 | Independent raw-sample replay preserves both cost passes and misses; ADR records the decision | `benchmarks/agent-triage/` |

The [frozen protocol](../benchmarks/agent-triage/protocol.json) defines numerical
cost gates; functional success alone is not cost acceptance. Keep workloads,
storage identity, sampling scope and thresholds explicit. Historical report
fixtures live in `benchmarks/agent-triage/fixtures/`; they support deterministic
tool tests, not a claim that current binaries reproduce an old performance result.

Record exact source/dirty state, relevant digests, tool and package identities,
platform, effective limits, commands, expected/observed results and cleanup status.
Commit reviewed summaries and reproducible tools; retain full failed/successful
run records under the evidence index's archive policy. Do not replace historical
failures with later passes or treat a local archive as a published artifact.

The stopping rule remains one bounded workflow with reproducible correctness,
recovery, authorization, delivery and cost evidence. Reopen abstraction work only
for independent real applications or a concrete capability gap supported by
measurement and a narrow architecture proposal.
