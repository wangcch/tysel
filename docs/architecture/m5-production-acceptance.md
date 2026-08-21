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
