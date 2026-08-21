# Production operations

This runbook is the deployment contract for Tysel Production v1. Linux x86-64
(`linux-x64`) and Linux arm64 (`linux-arm64`) are the supported production
targets. Treat the release archive, its reproducibility proof, signatures,
checksum, benchmark evidence, security evidence, and the deployment trust
policy as one release set.

## Release admission

Do not deploy an archive until all of these checks pass in an isolated staging
directory:

1. Obtain the archive and sidecars from the immutable tagged release. Obtain
   the trust policy through a separate authenticated configuration channel.
2. Compare the archive's SHA-256 digest with the single digest in
   `<archive>.sha256`.
3. Verify the archive, reproducibility-proof, benchmark-evidence, and
   security-evidence signatures. Use the canonical target for the host
   architecture.
4. Verify the reproducibility proof against the checked-out release
   `Cargo.lock`.
5. Extract only after verification. Confirm that `bin/tysel`,
   `bin/tysel-service`, and `bin/tysel-worker` are executable and that
   `share/acceptance/hello-service.evidence.json` exists.
6. Run the packaged acceptance service on a non-production port and exercise
   an application-owned readiness route.

Example verification for `linux-x64`:

```sh
archive=tysel-v1.0.0-linux-x64.tar.gz
expected=$(tr -d '[:space:]' < "$archive.sha256")
actual=$(sha256sum "$archive" | awk '{print $1}')
test "$actual" = "$expected"

./tysel release verify-artifact "$archive" --trust release-trust.json --target linux-x64
./tysel release verify-artifact "$archive.repro.json" --trust release-trust.json --target linux-x64
./tysel release verify-artifact benchmark-evidence-linux-x64.json \
  --trust release-trust.json --target linux-x64
./tysel release verify-artifact security-evidence-linux-x64.json \
  --trust release-trust.json --target linux-x64
./tysel release verify-reproducibility "$archive" \
  --evidence "$archive.repro.json" --lockfile Cargo.lock --target linux-x64
```

Use the verifier from an already trusted Tysel installation, not from the
unverified archive. The release workflow signs the archive, reproducibility
proof, benchmark evidence, and security evidence as `<artifact>.sig.json`.
The acceptance executable has its own `.sha256`, `.compat.json`,
`.sbom.cdx.json`, `.licenses.json`, and `.evidence.json` files inside the
archive; `tysel release verify` validates those files when an Evidence Index
signature is published for that executable.

Reject a release if the trust policy is expired, a key is revoked, a signature
or digest differs, the target is wrong, reproducibility verification fails, or
any required evidence is missing. Preserve the rejected set for investigation;
do not repair or regenerate publisher evidence on a deployment host.

## Deployment

Run the packaged application as an unprivileged dedicated account. Keep the
verified version in an immutable, versioned directory and switch a service
manager reference only after admission succeeds. The process working directory
is security-sensitive: packaged relative filesystem roots and SQLite paths are
resolved against it.

Before starting a service:

- review the embedded manifest with `tysel inspect --manifest tysel.toml` before
  building, and review the release `.compat.json` after building;
- expose only the configured `[server].listen` address, normally behind a TLS
  terminating reverse proxy;
- inject declared secrets and `TYSEL_POSTGRES_<NAME>` values through the service
  manager, never through the manifest, command line, TAP, or release artifact;
- when embedding the durable runtime with `PostgresStore`, inject
  `TYSEL_DURABLE_POSTGRES_URL` and use certificate-validated TLS in production;
- configure an OTLP endpoint only when a collector is reachable, and put
  credentials in the standard OTLP headers variables rather than endpoint
  userinfo or query parameters;
- grant the service account only its configured filesystem roots and, for an
  isolated profile on Linux, permission to use the intended cgroup v2 subtree.

Start a new version as a canary, wait for `tysel listen <address>`, and then
probe an application-defined readiness route through the same proxy path used
by clients. Tysel does not currently add a platform health endpoint; each
application must provide a bounded, side-effect-free route. Readiness should
test only dependencies needed to serve traffic. Use an external synthetic
check for end-to-end health.

Stop by sending the service manager's normal termination signal and allow the
configured grace period. Clean shutdown flushes OTLP providers and stops the
service-owned task plane. If the process does not exit before the deadline,
capture logs and process state before forcing termination.

## Configuration and secrets

Keep production configuration in a versioned deployment repository, with
secret values supplied by a secret manager. Record the release digest, trust
policy digest, application Evidence Index digest, target, manifest revision,
and deployment time in the change record. Never record connection URLs, OTLP
headers, bearer tokens, SQL, request bodies, or secret values.

`TYSEL_RELEASE_KEY_HEX` is a release-workflow secret only. Runtime hosts do not
need private signing material. Private seed files used by an offline signer
must be mode `0600` on Unix and destroyed from temporary storage after use.

For OTLP, setting `OTEL_SDK_DISABLED=true` disables export even when stale
endpoint variables remain. Endpoint values must be HTTP(S), no longer than 2
KiB, and contain no userinfo, query, or fragment. JSON logging and OTLP export
can be enabled independently.

## Signing-key rotation

1. Generate the new Ed25519 seed offline and obtain its public key and key ID
   with `tysel release key-info --key <path>`.
2. Publish a trust policy containing both old and new keys as `active`. Its
   validity must be at most 90 days, and consumers must prevent policy rollback.
3. Confirm all deployment verifiers have received the new policy.
4. Sign retained release artifacts with the new key and verify them from each
   deployment environment.
5. Mark the old key `retired` with a short `valid_until_unix` grace window.
6. After the grace window and fleet migration, remove the old key. Mark a
   compromised key `revoked` immediately; retirement is not a compromise
   response.

Rotate before trust-policy expiry. Keep offline audit records of key IDs,
policy digests, activation, retirement, and revocation times, but never copy a
private seed into those records.

## Durable Postgres backup and restore

Postgres is the production durable backend for library integrations using
`PostgresStore::connect_from_env`. The packaged service CLI does not
automatically start a durable Postgres dispatcher; the embedding service owns
dispatcher lifecycle and health checks.

Use the database platform's encrypted backups and point-in-time recovery. A
backup must include all Tysel durable tables, metadata, programs, events,
wakeups, signals, task locks, and statistics from one consistent database
snapshot. Do not restore individual task rows or selected tables: event
sequence, wakeup generation, leases, inbox state, and program identity form one
consistency boundary.

Backup procedure:

1. Stop or drain all writers, or use a database-native transactionally
   consistent snapshot that covers the complete Tysel schema.
2. Record the source release digest, database server version, database identity,
   backup digest, and the `schema_version` value in `tysel_durable_metadata`.
3. Encrypt the backup, restrict access, and test restoration on the configured
   retention schedule.
4. Resume writers only after the backup job reports durable completion.

Restore procedure:

1. Block application traffic and durable schedulers from the destination.
2. Restore the complete snapshot or recover to one point in time.
3. Start the same Tysel release against an isolated destination and confirm its
   supported durable log version accepts the database. A newer schema fails
   closed.
4. Verify program digests, replay representative completed histories, deliver a
   test signal to a non-production task, and confirm exclusive wakeup claiming.
5. Take a post-restore backup, then admit schedulers before application traffic.

SQLite durable files are suitable for local development. If one must be moved,
stop all writers and copy it with a SQLite-consistent backup mechanism; a plain
copy of a live database is not a production recovery plan.

## Upgrade and rollback

The current writer emits TAP v2 and reads TAP v1 through v2. Mixed deployment
is allowed only when every artifact's machine-readable compatibility report
accepts the version being served and the Capability ABI imports remain within
the deployment-approved WIT contract. Unknown versions and fields fail closed.

Upgrade in this order:

1. Admit the new release and back up durable state.
2. Stop new durable claims on the old scheduler and let active leases finish or
   expire. Do not copy or synthesize lease tokens.
3. Deploy one canary using the existing database and production-equivalent
   capabilities. Test HTTP, task dispatch, durable replay, signals, denied
   capabilities, and telemetry redaction.
4. Increase traffic gradually while comparing error rate, latency, memory,
   database contention, and due-work age with the old version.
5. Complete rollout only after at least one lease-expiry/reclaim cycle and one
   clean process restart succeed.

For rollback, stop new-version schedulers, restore the previous immutable
artifact by its verified digest, and repeat admission and canary checks. Binary
rollback without database restore is allowed only when the previous runtime
accepts the current durable `schema_version` and no irreversible application
data migration occurred. Otherwise restore the pre-upgrade database snapshot
and reconcile external side effects before admitting traffic. Never downgrade
by editing `tysel_durable_metadata`.

## Capacity and resource sizing

Release CI rejects a hello-service artifact over 20 MiB, idle Linux PSS over
32 MiB, or median cold start over 15 ms. These are admission baselines, not
production capacity promises. Size from load tests using the real bundle,
capabilities, request bodies, task mix, and telemetry exporter.

Manifest defaults are 128 MiB isolate memory, 50 ms CPU per turn, 30 seconds
per request, 1,000 in-flight requests, and 16 MiB request and response limits.
Lower limits to the smallest values the application passes under peak load.
Leave host memory for the supervisor, native libraries, connection pools,
buffers, and the OTLP exporter; do not multiply the isolate limit alone to size
the host.

`PostgresStore` defaults to 16 pooled connections per process and permits an
explicit pool size from 1 through 128. Budget total database connections across
all replicas and leave capacity for migrations, backups, and operators. Scale
replicas only after verifying database lock wait, due-work age, external API
quotas, and downstream connection limits. Multiple Postgres schedulers may
share a store; row locks, generation-fenced leases, and `SKIP LOCKED` claims
coordinate ownership.

## Monitoring and alerts

Enable OTLP traces and metrics through a nearby collector. Tysel emits:

- `tysel.http.server.requests` and `tysel.http.server.duration`;
- `tysel.capability.calls` and `tysel.capability.duration`;
- `http.server.request` and `tysel.capability` spans.

Alert on sustained HTTP 5xx rate, request latency relative to the configured
timeout, capability `error` or unexpected `denied` results, process restarts,
memory pressure, collector export absence, and database connection or lock
pressure. Monitor Postgres availability, saturation, backup age, replication
lag, storage, transaction conflicts, and certificate expiry with database
native tooling. The runtime does not yet export a due-wakeup-age metric; an
embedding scheduler should publish it, and operators should treat growth as a
durable-plane availability failure.

JSON logs are metadata-focused, but access must still be restricted. They omit
query strings, headers, bodies, SQL, filesystem paths, URLs, and secret values.
Metrics deliberately omit request IDs; use the process-local correlation ID on
spans and JSON logs for bounded investigation. Alert rules must use the fixed
low-cardinality labels and must not derive labels from user input.

## Incident response

For every incident, preserve the artifact and Evidence Index digests, target,
trust-policy digest, deployment revision, affected task IDs, correlation IDs,
database timeline, and redacted logs. Do not copy secrets or request payloads
into the incident record.

- **Signing-key compromise:** publish an authenticated policy marking the key
  `revoked`, stop deployments signed only by that key, rotate to a new offline
  key, re-sign trusted retained artifacts, and investigate policy rollback.
- **Suspected secret exposure:** revoke the credential at its provider, replace
  the secret-manager version, restart affected processes, and search only
  metadata for scope. Do not replay captured requests containing the secret.
- **Database outage or contention:** stop new dispatch, preserve current
  processes until leases expire, restore database service, then watch fenced
  reclaim and due-work age. Never force completion with a stale lease token.
- **Replay or history conflict:** quarantine the task, retain its complete
  history and program digest, stop automatic retries for it, and reproduce on a
  copy. Do not edit events, sequence numbers, or program bindings in place.
- **Isolate crash or resource kill:** keep the supervisor alive, inspect the
  bounded error metadata and host cgroup pressure, then reduce concurrency or
  fix the workload. Do not relax seccomp, Landlock, or capability grants as a
  first response.
- **Telemetry outage:** keep serving only if independent health and audit logs
  meet policy, restore the collector, and verify export with a synthetic
  request. OTLP authentication values must not be added to endpoint URLs.

After recovery, run the release acceptance path and a durable restart/replay
test before closing the incident. Convert any parser or protocol crash into a
non-sensitive regression corpus input and retain the relevant release evidence.
