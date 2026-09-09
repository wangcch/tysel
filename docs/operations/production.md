# Production operations reference

Choose the executable or OCI artifact boundary in
[Deployment selection](deployment.md) before applying this runbook.

Supported production application targets:

| Target | Operating system | Architecture |
| --- | --- | --- |
| `linux-x64` | Linux | x86-64 |
| `linux-arm64` | Linux | arm64 |

Builds default to the host target. A managed Tysel installation can build these
Linux artifacts from another supported macOS or Linux host by selecting
`--target linux-x64` or `--target linux-arm64`; it authenticates and caches the
same-version official target runtime.

## Build admission

Run these commands from the application project:

```sh
tysel doctor --install
tysel config validate
tysel task verify
tysel inspect
tysel build --release --target linux-x64 --output dist/APP
```

Admit the release only when every item passes:

| Gate | Required result |
| --- | --- |
| Installation | `tysel doctor --install` reports one valid three-binary release. |
| Project tasks | `tysel task verify` succeeds. |
| Authority | `tysel inspect` contains only approved capabilities. |
| Target | `APP.evidence.json` records the deployment target. |
| Artifact set | Executable plus checksum, compatibility, SBOM, licenses, and evidence files are present. |
| Signature | `tysel release verify APP --trust TRUST.json` succeeds when signing is required. |
| Health | The exact artifact starts on a staging port and its application readiness route succeeds. |

Store the executable and sidecars as one immutable set. For containers, bind
the final image digest to the admitted executable digest in the deployment
record. See [Reproducible release](../guides/reproducible-release.md).

## Runtime contract

| Setting | Production value |
| --- | --- |
| Account | Dedicated unprivileged user; no shared application account. |
| Artifact path | Immutable version or digest directory. |
| Working directory | Stable and writable only where relative filesystem or SQLite paths require it. |
| Listener | Manifest `[server].listen`; normally behind a TLS-terminating proxy or ingress. |
| Startup signal | Log contains `tysel listen <address>`. |
| Readiness | Application-owned, bounded, side-effect-free HTTP route. |
| Shutdown signal | Service manager sends its normal termination signal. |
| Shutdown grace | At least the application request timeout; 35 seconds for the default 30-second timeout. |

The production host runs the packaged executable. It does not need Tysel,
Node.js, npm, or the application source tree.

## Configuration and secrets

| Value | Source | Never store in |
| --- | --- | --- |
| Manifest configuration | Versioned deployment repository | Ad-hoc host edits |
| Declared secrets | Service manager or secret manager | Manifest, CLI arguments, release artifact |
| PostgreSQL grant `NAME` | `TYSEL_POSTGRES_<NAME>` | Manifest or image |
| Durable PostgreSQL URL | `TYSEL_DURABLE_POSTGRES_URL` | Manifest or image |
| OTLP credentials | Standard OTLP header variables | Endpoint userinfo or query string |
| Signing private key | Offline release system | Runtime host, repository, build log |

Set `OTEL_SDK_DISABLED=true` to disable export. OTLP endpoints must be
HTTP(S), no longer than 2 KiB, and contain no userinfo, query, or fragment.

Record only deployment metadata:

- source commit and manifest revision;
- executable, evidence, and image digests;
- target and Tysel version;
- signing key ID and trust-policy digest;
- deployment revision and time.

Do not record credentials, connection URLs, headers, SQL, request bodies, or
secret values.

## Health and shutdown

Startup:

1. Start the admitted artifact on a non-production port or canary.
2. Wait for `tysel listen <address>`.
3. Probe the readiness route through the same proxy path clients use.
4. Admit traffic only after the probe succeeds.

Readiness should check only dependencies required to serve traffic. Use an
external synthetic check for end-to-end health; Tysel does not add a platform
health endpoint.

Shutdown:

1. Stop admitting new traffic and durable claims.
2. Send the normal service-manager termination signal.
3. Wait through the configured grace period.
4. If the process remains, capture logs and process state before forcing it.

On SIGTERM or Ctrl-C, packaged services and `tysel run` close the listener,
stop new durable claims, and drain HTTP/1 and HTTP/2 connections. Idle sockets
close immediately. WebSockets receive close code 1001 (Going Away) when the
transport can send it. Connections that remain after `request_timeout_ms + 1000` milliseconds are closed forcibly. This HTTP grace is shared across
connections; task-plane shutdown and telemetry flushing can add time, so keep
the service-manager grace longer. Development servers use the same draining
path, with enough grace for the largest timeout seen during reloads.

Clean shutdown flushes OTLP providers and stops the service-owned task plane.

Durable storage availability errors move the scheduler to `recovering`, with
exponential retry from 200 ms to a 5-second base plus at most 25% jitter. New
durable starts fail while recovery is pending. A successful poll restores
`healthy`. Unrecoverable store/scheduler failures, such as a corrupt program
digest, reach the service owner, which drains and exits unsuccessfully.
Completion-write availability failures retain the JSON outcome in memory and retry
persistence without rerunning the handler while the execution lease is valid.
If that authority expires or is superseded, the pending handle and its slot are
released without reporting completion. The scheduler can reclaim the original
managed task and replay its history; recorded effects do not execute again.
At most 32 active executions and pending
finalizations share reserved slots. Pending entries retain compact results and
lease/sequence metadata, releasing consumed replay history. `completion_pending`
logs the affected task ID. Completed outcomes are still reported when a different
task in the same scheduling batch encounters a storage failure.
Retries remain subject to the original lease. Completion transactions check fresh
time after acquiring their locks and again before commit; expiry rolls back both
the result and quota changes. An identical already-committed outcome can still be
acknowledged after expiry. Shutdown reports failure if outcomes still await
persistence. Execution eligibility survives process death, and stale executors
cannot write history, suspension or completion records. An interrupted effect
without a recorded outcome can run again; reconcile external outcomes or use
provider idempotency before repeating writes.

For admission and signal acknowledgement gaps, use the optional stable
`idempotencyKey` and an application outbox. The runtime atomically records each
keyed request in its own database; it cannot atomically commit the application's
separate database. See the [control contract](../reference/runtime/durable.md#control-api).

Per-task execution failures are logged as `task_failed`; they do not by
themselves stop unrelated work. These states
and task outcomes are bounded `durable` log events containing task IDs and
attempt counts, without payloads, source, or database credentials. An HTTP-only
health route may still succeed during recovery; use scheduler diagnostics for
durable health.

### Durable log v3 upgrade

This release migrates durable log v1/v2 to v3 transactionally on store open. The
application manifest schema remains version 1. Before upgrading:

1. Drain and stop **all** old writers and schedulers.
2. Back up the complete durable store and verify that it can be restored.
3. Start the new release or its admin command; verify replay and completion.
4. Resume traffic only after those checks pass.

Do not run old and new writers together. Old runtimes reject v3 on startup;
rollback requires restoring the pre-upgrade snapshot and its matching release.
Existing v1/v2 programs remain active unless a new execution explicitly completes
them. An absent wakeup is not proof of completion. Completion results and the
active catalog counters persist across reopen. Retained data is removed only
by [`tysel durable prune`](../reference/cli/tasks.md#tysel-durable).

Failed executions are retained for investigation. Legacy interrupted runs without
a wakeup or a v3 execution record remain unclassified and are not automatically
replayed. Do not infer a safe replacement write from absent runtime history.
Completion pruning also removes admission identities and signal receipts; keep
retention longer than the application outbox retry horizon.

## Durable Postgres backup and restore

Use PostgreSQL for production durable state. SQLite is a local or single-writer
storage option, not a shared multi-replica durable backend.

### Backup

1. Drain all writers, or take one database-native transactionally consistent
   snapshot of the complete Tysel schema.
2. Record the source release digest, database identity and version, backup
   digest, and `tysel_durable_metadata.schema_version`.
3. Encrypt the backup and restrict access.
4. Test restoration on the retention schedule.
5. Resume writers only after durable backup completion.

The backup must contain all Tysel durable tables, including metadata, programs,
events, wakeups, signals, completions, execution leases, idempotency receipts,
task locks, and statistics. Do not copy selected task
rows or individual tables.

### Restore

1. Block application traffic and durable schedulers.
2. Restore the complete snapshot or recover to one point in time.
3. Start the same Tysel release against an isolated destination.
4. Verify schema acceptance, program digests, representative replay, signal
   delivery, and exclusive wakeup claiming.
5. Take a post-restore backup.
6. Admit schedulers, then application traffic.

For SQLite, stop every writer and use a SQLite-consistent backup mechanism. A
plain copy of a live database is not a recovery plan.

## Upgrade and rollback

Upgrade sequence:

1. Admit the new release and back up durable state.
2. Stop new durable claims on old schedulers; let active leases finish or
   expire.
3. Deploy one canary against production-equivalent capabilities and storage.
4. Test HTTP, task dispatch, durable replay, signals, denied capabilities, and
   telemetry redaction.
5. Increase traffic while comparing errors, latency, memory, database
   contention, and due-work age.
6. Complete rollout after a lease-expiry/reclaim cycle and clean process
   restart succeed.

Rollback sequence:

1. Stop new-version schedulers.
2. Restore the previous immutable artifact or image by verified digest.
3. Repeat admission, canary, and readiness checks.
4. Restore the pre-upgrade database only when the previous runtime does not
   accept the current schema or the application migration is irreversible.

Never edit `tysel_durable_metadata`, synthesize lease tokens, or recreate an
old image under a mutable tag.

## Capacity defaults

These manifest defaults are limits, not capacity promises:

| Limit | Default |
| --- | ---: |
| Isolate memory | 128 MiB |
| CPU per turn | 50 ms |
| Request timeout | 30 seconds |
| In-flight requests | 1,000 |
| Request body | 16 MiB |
| Response body | 16 MiB |
| Application PostgreSQL pool | Up to 4 connections per process; separate from durable PostgreSQL storage |

Load-test the exact release with production request bodies, task mix,
capabilities, dependencies, and telemetry. Budget total database connections
across replicas and leave capacity for migrations, backups, and operators.

## Monitoring and alerts

| Signal | Alert condition |
| --- | --- |
| `tysel.http.server.requests` | Sustained 5xx rate or traffic disappears unexpectedly. |
| `tysel.http.server.duration` | Latency approaches the configured request timeout. |
| `tysel.capability.calls` | Unexpected `denied` or sustained `error` results. |
| Process state | Restart loop, forced termination, or memory pressure. |
| OTLP export | Collector export disappears while traffic continues. |
| PostgreSQL | Availability, pool saturation, lock pressure, replication lag, storage, backup age, or certificate expiry. |
| Durable scheduler | Due-work age grows or reclaim stops. |

Tysel JSON logs omit query strings, headers, bodies, SQL, filesystem paths,
URLs, and secret values. Restrict access anyway. Use the process-local
correlation ID in spans and logs; do not create metric labels from user input.

See [Observability](../guides/observability.md) for exact signals and redaction.

## Incident response

Preserve artifact and evidence digests, target, trust-policy digest, deployment
revision, affected task IDs, correlation IDs, database timeline, and redacted
logs.

| Incident | First response | Prohibited shortcut |
| --- | --- | --- |
| Signing-key compromise | Revoke the key in authenticated policy, stop affected deployments, rotate offline, and re-sign retained verified artifacts. | Keeping the key active to avoid deployment failures. |
| Secret exposure | Revoke at the provider, issue a new secret version, restart affected processes, and search metadata for scope. | Copying captured secrets or request bodies into the incident record. |
| Database outage or contention | Stop new dispatch, preserve processes until leases expire, restore service, then watch fenced reclaim and due-work age. | Completing work with a stale lease token. |
| Replay or history conflict | Quarantine the task and retain its complete history and program digest. | Editing events, sequence numbers, or program bindings. |
| Isolate crash or resource kill | Inspect bounded error metadata and host cgroup pressure; fix workload or concurrency. | Relaxing seccomp, Landlock, or capability grants first. |
| Telemetry outage | Use independent health and audit logs, restore the collector, and verify with a synthetic request. | Adding OTLP credentials to endpoint URLs. |

After recovery, rerun release admission and one durable restart/replay test
before closing the incident. See [Debugging](../guides/debugging.md).

### Active-crash and acknowledgement gates

From a checked-out runtime source tree, build the local binaries and run:

```sh
cargo build --locked -p tysel-cli -p tysel-runtime -p tysel-isolate --bins
python3 tests/p1/crash_recovery.py
```

The gate uses only loopback services and disposable databases. It covers six
SIGKILL windows in both `tysel run` and standalone mode with an empty PATH:
initial unacknowledged admission, before the external write, external commit
before response, response before effect recording, after effect recording, and
projection update before signal-wait registration. It also retries a persisted
signal after its HTTP acknowledgement is lost. Passing requires the same task
to complete with the reconciled result, a single external create, and a single
consumed approval event. The fake provider reconciles by stable operation ID;
the runtime does not make arbitrary provider writes exactly-once.

Run `cargo test --locked -p tysel-durable --test execution` with a disposable
`TYSEL_POSTGRES_TEST_URL` to also exercise PostgreSQL execution fencing and
idempotency. The SIGKILL gate above uses SQLite; PostgreSQL deployment crash and
capacity checks, and an application's own outbox integration, remain separate
release verification work.
