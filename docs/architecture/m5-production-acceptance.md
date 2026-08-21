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

`tysel build --release` emits three sidecars next to the single executable:

- `.sha256` contains only the lowercase SHA-256 artifact digest.
- `.compat.json` contains the M5.1 TAP compatibility report.
- `.evidence.json` binds the artifact digest, byte size, target, application
  identity, execution profile, and compatibility report under a versioned
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
