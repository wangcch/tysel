# Recoverable, bounded agent triage

A local, read-only example of a trusted Tysel service coordinating an isolated
TypeScript plugin. Two fake customer scopes and independently counted HTTP
fixtures exercise authorization and recovery without a model, paid call, real
credential or customer data.

For standalone binaries, persistent namespaces and stopped backup/restore, see
the [packaged deployment guide](DEPLOYMENT.md). `run.py` remains the disposable
source demo; its namespace is removed when the launcher exits.

## Run

Build matching tools from the current checkout:

```sh
cargo build --locked -p tysel-cli -p tysel-runtime -p tysel-isolate --bins
```

From the repository root:

```sh
python3 examples/agent-triage/run.py
```

Add `--audit` to include the bounded metadata audit trail in the printed result.

This starts the caller, isolated plugin, data fixture and admission dispatcher;
submits one job; prints `succeeded`, `Prioritize a-01: Checkout unavailable`, and
`dataSourceReads: 1`; then cleans up processes and temporary files. Python 3.9+
and a Unix host are required. `--bin-dir` selects matching tools elsewhere.
The launcher copies source and listens on ephemeral loopback ports.

For manual requests, keep the demo running and use the printed URL as `BASE_URL`:

```sh
python3 examples/agent-triage/run.py --serve
```

```sh
curl "$BASE_URL/jobs" \
  -H 'Authorization: Bearer demo-a' \
  -H 'Idempotency-Key: triage-001' \
  -H 'Content-Type: application/json' \
  -d '{"protocolVersion":1,"ticketIds":["a-00","a-01"]}'

curl "$BASE_URL/jobs/JOB_ID" -H 'Authorization: Bearer demo-a'
```

`demo-a` identifies `customer-a`, which owns `a-00` through `a-15`; `demo-b`
identifies `customer-b`, which owns `b-00` through `b-15`. Ticket `01` has the
highest priority. Empty input completes without reading a detail. These public
credentials are development identities, not production authentication.

## Execution and trust boundary

Schema creation and additive migration are idempotent across HTTP isolates.
Each request reads current authority and applies expiry, boot ownership and
delivery maintenance in that order. A failed initialization can be retried by
a later request. State transitions retain their transactional audit records.

```text
Client -> caller -> jobs.db: immutable admission and outbox intent
Local dispatcher -> caller -> Durable.start: stable job key and original bundle
Durable handler -> isolated plugin: projected snapshot, request operation
Durable handler -> jobs.db: reserve physical attempt before dispatch
Durable handler -> data fixture: fresh authorized read with opaque host secret
Durable handler -> jobs.db: accept the current attempt's projected result
Durable handler -> isolated plugin: recorded detail, request summary
Durable handler -> jobs.db: conditional immutable business result
Durable runtime -> durable-events.db: effects and eventual completion
Local dispatcher -> jobs.db: reconcile acknowledgement/completion under same key
```

Authorization, budget, ownership and result validation live in the trusted
service. The Python launcher supervises local processes, supplies the fixture
and repairs admission/completion acknowledgements through private endpoints.
The runtime's existing Durable scheduler handles continuation and lease recovery.
Client disconnect does not cancel an admitted job. The launcher remains part of
this example; it is not a packaged deployment or generic broker.

One caller process owns the application SQLite connection. Its HTTP pool has
nine isolates, leaving admission/status capacity while eight control requests
wait on work. Durable execution also uses the runtime's managed execution path.
The 32 MiB heap limit is per isolate, not a total process RSS promise. Multiple
caller processes or independent schedulers on the same SQLite files are outside
this example's contract.

The isolated plugin receives an application envelope with `protocolVersion`,
`jobId`, `stepId`, `attemptId`, and `payload`. The caller validates every field
and operation against its own state. Only one logical `ticket.read` is allowed.
It never forwards a plugin-supplied origin, credential or authorization decision.
The data fixture independently checks ownership and counts physical requests;
its private note is projected out before persistence or plugin continuation.
The dedicated `plugin.toml` grants no network, filesystem or secret access;
the plugin manifest is included in the pinned plugin digest. A summary is
untrusted text: even instructions or URLs in it cannot trigger another operation.

## API and bounds

| Behavior | Contract |
| --- | --- |
| Submission | `POST /jobs`, exact body `{protocolVersion:1,ticketIds:string[]}`, required visible-ASCII key of 1–128 characters |
| Idempotency | Same customer, key and canonical input returns the same job; conflicting input returns 409; separate customers may use the same key |
| Lookup | `GET /jobs/:jobId`; another customer's job returns 404; invalid identity returns 401 |
| Logical operations | One selection, at most one detail read and one summary; empty input needs selection only |
| Physical attempts | At most two reserved attempts for each logical operation; unknown outcomes consume their slot |
| Retry policy | Transport failures, timeouts, 429 and 5xx may retry once after 250 ms; other HTTP errors, redirects and invalid protocol/results are terminal |
| Bytes | Admission/plugin request 16 KiB; plugin response 8 KiB; adapter response 16 KiB, checked before JSON parsing |
| JSON | Strict UTF-8, no duplicate object members (including escaped aliases), maximum nesting depth 32 |
| Text/snapshot | Up to 16 distinct authorized tickets; subject at most 500 UTF-16 code units; summary at most 1,024 UTF-16 code units |
| Time | 5 seconds per call; original job lifetime 120 seconds, including downtime, leases and backoff |
| Capacity | Four active jobs per customer, eight total; new submissions beyond capacity return 429 |
| Retention | 1,000 jobs per namespace; new keys return 503 at capacity; retained identities still resolve |
| Storage retry | Three bounded application-boundary attempts with 250 ms delay; never refill physical-attempt slots |

Redirect responses are requested in `manual` mode and rejected before another
hop. No implicit redirect request consumes the allowance. A timeout cannot undo
an already dispatched read. Two reservations can therefore correspond to zero,
one or two physical reads; this is not an exactly-once read guarantee.

## Recovery, ownership and storage

New jobs use execution version 2. States are `accepted`, `running`, `recovering`,
`succeeded`, `failed`, and `expired`. Restart changes unfinished version-2 jobs
to `recovering`; the Durable scheduler waits for the existing lease before
replaying the original program. With the normal 40-second request timeout,
the runtime lease is 45 seconds. Tests shorten the request timeout to 1.5 seconds
and wait for the resulting 6.5-second lease; they never edit lease timestamps.

Application state and runtime history are separate SQLite files. Business
admission itself is the outbox intent; the dispatcher calls `Durable.start`
with `triage.v2:<jobId>` and the immutable `{jobId}` input. A lost acknowledgement
is retried using that same key and full bundle. There is no transaction spanning
these stores. A committed business result may precede runtime completion; the
acceptance suite verifies that the eventual completion returns the same result.
`task_id` and `delivery` record acknowledgement/completion separately from the
business status. Public status returns the business result.

The application journal reserves `(job, logical step, ordinal)` atomically and
accepts a result only under the current application owner. A fresh owner is
allocated on each runtime entry, not replayed from a historical step. Old
responses and terminal writes cannot replace the current accepted state. This
uses existing SQLite and Durable APIs, without exposing private runtime lease
tokens or adding orchestration APIs.

An unrecorded read may observe a changed subject when retried. Once the current
attempt's projected result is accepted in the application journal, replay reuses
it even if runtime effect recording failed. Permission, original deadline,
program/plugin identity and adapter identity are checked again at each new
physical dispatch. A revoked or moved ticket cannot be read on recovery.
Revocation during Durable backoff prevents the next read. It cannot erase an
already accepted detail, or cancel a request that passed the dispatch check and
is already in flight. Such a response may still be accepted and summarized
within the original deadline and ownership. Stronger remote revocation requires
the data source itself to enforce current policy at the time of access.

## Bounded audit trail

`jobs.db` also holds `triage_audit`: job, step, operation, physical-attempt ordinal
and ID, decision, fixed outcome code and timestamp. It contains no request body,
subject, summary, credential, raw idempotency key or raw error. Admission,
reservation, accepted/denied outcome and terminal events are SQLite triggers in
the same transaction as the corresponding business mutation. If audit storage
fails, the mutation rolls back; an unaudited reservation cannot dispatch work.
Authority denials before a reservation have a separate deduplicated event.

The bound is 20 rows per newly admitted job: one admission, up to six
reservations, six outcomes, six authority denials and one terminal event.
Duplicate submissions and replay do not append duplicate events. The 1,000-job
namespace therefore has at most 20,000 audit rows. Unauthorized HTTP requests
are not appended to this table. Existing pre-P3 history is not backfilled.
This is a local operational record, not an immutable external audit service.

Runtime-history storage failures retain the runtime's lease/fencing recovery
behavior. Application boundary storage failures use bounded recorded retries;
exhaustion becomes `STORAGE_UNAVAILABLE` when the business store can record it.
An outage that also prevents admission, ownership or terminal writes can leave
an unresolved task rather than a fabricated success. Once storage is available,
the deadline sweep expires unfinished business work. The dispatcher retries
pending acknowledgements until the original deadline plus 120 seconds, then
marks delivery `unresolved` for inspection; it never substitutes a new task key.
Pending records rotate by their last check time so unavailable older admissions
cannot monopolize the dispatcher.
Retained unresolved records are not automatically revived.

## Program identity, migration and retention

Each launcher invocation pins a complete caller-source copy for its temporary
namespace. Caller restarts restore that original copy before draining outbox
work. This deliberately excludes in-place caller deployment within a namespace:
use a new namespace for new source. The runtime independently retains the
original bundle for tasks it already admitted. Plugin/adapter incompatibility
fails pending work with `VERSION_MISMATCH` before a new external dispatch.
This local convention is not a general rolling-deployment system.

The additive schema migration keeps P1 terminal results unchanged. Existing
unfinished P1 rows still become `failed/INTERRUPTED`; they do not silently adopt
P2 retries. New P2 rows recover with their original limits and deadline.

No records, runtime receipts or source history are pruned automatically. A full
launcher exit deletes the temporary namespace. Thus same-key protection lasts
for the namespace lifetime, with no deduplication across launcher invocations.
Backup, production identity, multiple replicas, cancellation and external writes
remain outside scope.

## Acceptance

Contract, recovery and adversarial cases are integrated into smoke/full/release suites:

```sh
python3 tests/acceptance/run.py --suite smoke \
  --case agent-triage --case agent-triage-recovery --case agent-triage-adversarial \
  --bin-dir target/debug --profile debug --target darwin-arm64 \
  --output target/agent-triage-acceptance
```

Choose the actual host target and a new output directory. The parent report
records source/binary fingerprints; case reports record observations and cleanup.
Recovery tests use temporary source instrumentation, disposable SQLite failure
triggers and an optional counted plugin proxy. These fault routes and barriers
are never added to the example's checked-in TypeScript endpoints.

Coverage includes retained identity, isolation, byte/capacity limits, the P2
crash matrix, changing reads, stale ownership, revocation, retry exhaustion,
program pinning, storage/completion gaps and P1 migration. P3 adds separate
malicious worker fixtures, serial/concurrent swapped responses, earlier-attempt
responses, two-customer crash/capacity tests, streamed byte limits, Unicode text
limits, permission probes, secret canaries and audit transaction failures.
macOS results establish local development behavior. The separate
`agent-triage-deployment` case covers P4 packages, retained state and Linux setup
failures. P5 architecture/cost decisions and production identity remain open.
