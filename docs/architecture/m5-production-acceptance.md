# M5 Production v1 acceptance

M5 turns the runtime milestones into a release contract. Completion requires
the following evidence to be generated and verified in CI:

1. TAP and Capability ABI versions have documented reader/writer compatibility
   windows. Unknown versions, profiles, fields, and compatibility flags fail
   closed rather than silently selecting a more privileged mode.
2. Every release publishes a deterministic machine-readable compatibility
   report, benchmark evidence, security and fuzzing results, an SBOM, checksums,
   signatures, license data, and a release evidence index.
3. Release artifacts are reproducible from a pinned toolchain and dependency
   graph. Evidence identifies the source commit, target, commands, and artifact
   digests without embedding timestamps in reproducibility inputs.
4. Linux x86-64 and arm64 release artifacts pass the same runtime, isolation,
   package, upgrade, and rollback acceptance suite.
5. Durable Postgres storage and OTLP export preserve the existing ownership,
   replay, capability-audit, and metadata-redaction contracts.
6. Production documentation covers deployment, key rotation, backup/restore,
   upgrades, rollback, resource sizing, monitoring, and incident response.

## M5.1 TAP compatibility contract

The first M5 slice freezes the existing TAP v2 envelope as the current writer
format and keeps TAP v1 as the minimum readable legacy format. The envelope and
manifest versions must match. Runtime versions must be semantic versions, and
the only accepted execution profiles are `service`, `isolated`, and
`component`. Unknown manifest and component-index fields are rejected.

`tysel-package` exposes a deterministic JSON-serializable compatibility report
with the report schema version, compatibility decision, exact TAP version,
supported version window, runtime version, execution profile, sorted Component
ABI versions, and bounded validation issues. Current and legacy-valid payloads
are compatible; older, newer, malformed, or ambiguous payloads are not.

This report is the compatibility input for the later SBOM, signature, and
Release Evidence Index slices. It intentionally contains no timestamp or
host-specific value so identical TAP payloads produce identical reports.

## M5.2 Release evidence index

`tysel build --release` emits five sidecars next to the single executable:

- `.sha256` contains only the lowercase SHA-256 artifact digest.
- `.compat.json` contains the M5.1 TAP compatibility report.
- `.sbom.cdx.json` is a deterministic CycloneDX 1.5 inventory whose metadata
  component binds the final executable SHA-256 digest.
- `.licenses.json` contains the normalized SPDX license expression for every
  production component.
- `.evidence.json` binds the artifact digest, byte size, target, application
  identity, execution profile, compatibility report, SBOM digest, license
  inventory digest, and embedded runtime-inventory digest under a versioned
  schema.

These files are deterministic for identical inputs and contain no timestamp or
absolute build path. Later signed attestations may add commit, builder, and
timestamp provenance while referencing the immutable evidence-index digest;
that non-reproducible provenance stays outside the reproducible index itself.
Compatibility and application identity are derived by reopening the final
executable and validating its embedded TAP, not from pre-embed build state.
Sidecars are staged in the output directory and `.evidence.json` is published
last as the commit marker; failed publication leaves no authoritative index.
Non-release builds do not emit release sidecars. Evidence and compatibility
schemas reject unknown fields so newer security semantics cannot be ignored by
older verifiers.

## M5.3 SBOM and license gate

The checked-in runtime inventory is generated from `cargo metadata --locked`.
Its production roots are `tysel-cli`, `tysel-runtime`, and `tysel-isolate`;
normal and build dependencies are followed across supported targets, while
dev-only edges are excluded. Registry source checksums come from `Cargo.lock`.
Generation fails closed when a reachable package lacks a license expression,
has no lock entry, produces a duplicate package URL, or cannot be resolved.
Legacy slash-separated dual-license declarations are normalized to SPDX `OR`.

The inventory is sorted by package URL and contains the `Cargo.lock` digest,
but no timestamps, host paths, or network-derived mutable fields. CI runs
`cargo run --locked -p tysel-build --bin tysel-supply-chain -- --check`, which
regenerates the inventory and fails on drift. A release build consumes the
checked inventory without invoking Cargo or requiring network access.

`verify_release_evidence` re-hashes the executable, SBOM, license inventory,
and checksum sidecars; compares the compatibility sidecar to the index; parses
all JSON with strict schemas; verifies that the SBOM identifies the executable;
and rejects a runtime-inventory mismatch. `.evidence.json` remains the final
commit marker, so partially published sidecars are never authoritative.

## M5.4 Offline signatures and key rotation

`tysel release sign <artifact> --key <path>` first verifies every unsigned
release sidecar, then writes `.evidence.sig.json`. The signature uses Ed25519
and a domain-separated message containing the derived key ID, signed Unix
time, and SHA-256 digest of the exact `.evidence.json` bytes. Private keys are
32-byte seeds encoded as 64 lowercase hexadecimal characters. On Unix, key
files must deny all group and other permissions; keys are never accepted on a
command-line argument or environment variable.

`tysel release key-info --key <path>` prints the public key and its key ID. The
key ID is the full SHA-256 digest of the 32-byte public key, so aliases cannot
redirect a signature to another key. The `tysel release verify` command accepts
an artifact and `--trust <policy.json>`, verifies the artifact and all unsigned
evidence, then applies a strict trust policy and Ed25519 signature check.

Trust-policy keys are sorted by key ID and have `active`, `retired`, or
`revoked` status plus validity bounds. Policies have a mandatory expiration to
bound stale-policy use; their declared lifetime cannot exceed 90 days.
Verification uses both the signed issue time and the current time: a retired
key is accepted only during its explicit grace window, and a revoked key is
rejected unconditionally. Rotation therefore proceeds by
publishing the new active key during an overlap window, re-signing retained
artifacts, marking the old key retired with a short deadline, and finally
removing or revoking it. Compromise requires `revoked`, not `retired`.

Deterministic multi-architecture archives use the same keys and trust policy
through `tysel release sign-artifact <archive> --target <target> --key <path>`
and `tysel release verify-artifact <archive> --trust <policy.json>`. The target
and exact archive SHA-256 digest are domain-separated from Evidence Index
signatures, preventing either signature type from being replayed as the other.

Generate a seed with a cryptographic random source and immediately restrict it,
for example `openssl rand -hex 32 > release.key` followed by `chmod 600
release.key`. The trust policy is itself a deployment trust anchor: distribute
it through an authenticated configuration channel, refresh it before
`expires_at_unix`, and prevent rollback at that layer.

## M5.5 Security audit and fuzzing gates

CI treats known RustSec advisories, yanked dependencies, unapproved licenses,
and unknown registries or Git sources as release blockers. `cargo-audit` runs
with warnings denied, and `cargo-deny` evaluates the complete locked dependency
graph for all supported targets and features. Workspace path dependencies are
allowed because they are reviewed source in this repository; registry and Git
dependencies remain source-restricted. Duplicate dependency versions are
reported for maintenance without weakening the advisory gate. Advisory ignores
must not be added to make CI green: a vulnerable dependency is upgraded or the
affected feature is removed.

The fuzz workspace is independently locked and pins the CI nightly and
`cargo-fuzz` version. Five targets exercise the externally controlled parsing
boundaries: TAP decoding and round trips, manifest and grant parsing, isolate
IPC framing, task RPC framing, and release evidence/signature/trust metadata.
Every pull request runs 10,000 bounded executions per target with per-input
timeouts and an RSS limit. This smoke suite is a deterministic release gate,
not a replacement for longer campaigns. Before a release candidate, run each
target with a time budget and retain any minimized regression input, for
example:

```sh
cargo +nightly-2026-08-15 fuzz run tap_decode -- -max_total_time=3600 -timeout=10 -rss_limit_mb=4096
```

Crashing inputs are promoted to named corpus fixtures only after confirming
that they contain no secret or customer data. The root and fuzz lockfiles must
both be regenerated after security-driven dependency upgrades, and the runtime
SBOM inventory must be regenerated before the change is accepted.

## M5.6 Benchmark, reproducibility, and multi-architecture release

The release toolchain is pinned to Rust 1.97.1. Linux x86-64 and arm64 run the
same formatting, inventory, Clippy, workspace-test, packaging, reproducibility,
and benchmark gates. Each target is built twice into independent Cargo target
directories with the source path remapped, the linker build ID disabled, and
`SOURCE_DATE_EPOCH` derived from the source commit. Both trees package the same
hello-service acceptance artifact and all of its release sidecars before a
stable-order, stable-owner, stable-mtime archive is compressed without a gzip
timestamp.

`tysel release reproduce` rejects unequal archive bytes and writes a strict
machine-readable proof containing the source commit, canonical target, exact
toolchain, both build commands, archive digest and size, `Cargo.lock` digest,
and embedded runtime-inventory digest. The supplied lockfile must match the one
bound into the runtime inventory. Only `linux-x64` and `linux-arm64` are valid
production targets, and ambiguous, multiline, oversized, or unknown provenance
fails closed. `tysel release verify-reproducibility` re-hashes the archive,
lockfile, and embedded inventory and validates both recorded build entries.

The benchmark artifact records all 11 cold-start samples, p50 and gate result,
idle PSS, binary size, CPU, OS, architecture, command, source commit, and
artifact digest. Tagged builds publish both target archives, checksum files,
detached Ed25519 signatures, reproducibility proofs, and benchmark evidence.
The signing key is mandatory for the release workflow and is supplied only as
the protected `TYSEL_RELEASE_KEY_HEX` secret; the CLI still receives a
permission-restricted temporary key file. The archives also retain a packaged
hello-service and its compatibility, SBOM, license, checksum, and Release
Evidence Index sidecars as an executable acceptance fixture. Signatures are
created after deterministic comparison; signing timestamps are deliberately
excluded from the archive and reproducibility comparison.
Both the archive and its reproducibility proof receive detached signatures, so
the build commands and source provenance cannot be replaced independently of
the published bytes.

## M5.7 Postgres Durable Store

The durable engine and scheduler consume the backend-neutral `DurableStore`
contract. `SqliteStore` remains the local implementation; `PostgresStore` is
the production implementation and uses a bounded connection pool. Existing
`DurableSession`, `DurableDispatcher`, and persistent program-catalog callers
therefore exercise the same replay and ownership path with either backend.

Postgres mutations take an exact per-task row lock before allocating a history
sequence or changing signal state. Event/history accounting, event+wakeup,
signal enqueue+wakeup, and signal consume+event append are single database
transactions. Due-task acquisition uses `FOR UPDATE SKIP LOCKED`; completion,
renewal, and release compare the complete lease token so a stale worker cannot
modify a newer generation. The schema enforces unsigned-range checks where
Postgres permits them, fixed-size task IDs and digests, immutable program
identity, bounded history, inbox, and program catalogs, and the same durable
log version as SQLite.

Production credentials are host-only. Set `TYSEL_DURABLE_POSTGRES_URL` and call
`PostgresStore::connect_from_env`; the URL is not represented in the manifest,
TAP, logs, or durable history. TLS is selected by the PostgreSQL `sslmode`
setting. `sslmode=disable` is intended only for a trusted local test service;
production deployments use certificate-validated TLS.

CI supplies `TYSEL_POSTGRES_TEST_URL` to a live parity test. The test verifies
concurrent sequence compare-and-swap, deterministic replay history, exclusive
claim ownership and completion, signal suspension/wakeup/consumption, and the
persistent module catalog. Without that variable the live test is skipped so
offline development remains deterministic; the release workflow always
provides its Postgres service.

## M5.8 OTLP export and metadata redaction

Packaged services enable OTLP/HTTP protobuf export only when the deployment
sets `OTEL_EXPORTER_OTLP_ENDPOINT` or a signal-specific
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` / `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`.
`OTEL_SDK_DISABLED=true` overrides stale endpoint configuration. Endpoints must
use HTTP or HTTPS, are bounded to 2 KiB, and reject URL userinfo, query strings,
and fragments; authentication belongs in the standard OTLP headers variables.
HTTPS uses the platform certificate store through the existing native-TLS
stack. No endpoint or authentication value enters TAP, application logs,
durable history, span attributes, or initialization errors.

The exporter emits `http.server.request` and `tysel.capability` spans plus four
metrics: `tysel.http.server.requests`, `tysel.http.server.duration`,
`tysel.capability.calls`, and `tysel.capability.duration`. Export is independent
of stderr JSON logging, and a process guard flushes both providers during clean
shutdown. The only HTTP dimensions are service name, sanitized method, numeric
status, and status class. Raw path, query, URL, headers, body, peer address, and
secret values are excluded. Capability, operation, and result labels use exact
allowlists; unknown values become `redacted`, including values that otherwise
look like valid alphanumeric labels. Metrics omit request IDs to avoid
high-cardinality series; spans may contain the process-local correlation ID.

The OTLP integration test runs a child runtime against a loopback fake
collector, receives real protobuf trace and metric exports, verifies both
signals are present, and scans the encoded payload for paths, SQL, URLs, and
bearer-token fixtures. Unit tests separately cover explicit signal activation,
SDK disable precedence, endpoint validation, and fail-closed label handling.

## M5.9 Production operations

The [Production operations runbook](../operations/production.md) defines the
release-admission, deployment, key-rotation, durable backup/restore,
upgrade/rollback, sizing, monitoring, and incident-response procedures. It
keeps the operational boundary explicit: release keys remain offline, runtime
secrets remain host-only, application readiness is application-owned, and the
embedding service owns durable Postgres dispatcher lifecycle. Restore and
rollback procedures preserve the event-log, wakeup, lease, signal, and program
catalog as one consistency boundary rather than modifying durable rows in
place.
